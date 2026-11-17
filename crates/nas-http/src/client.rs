//! HTTPS client bound to one NAS and its pinned certificate, and the first
//! contact that reads the certificate before anything is sent.

use std::error::Error as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nas_model::ProviderError;
use reqwest::{RequestBuilder, StatusCode, Url};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::Fingerprint;
use crate::verifier::{self, Seen};

/// Time limit for a whole request, from connect to the last body byte.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// Time limit for the TCP connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const USER_AGENT: &str = concat!("NASStats/", env!("CARGO_PKG_VERSION"));

/// HTTPS client for one NAS. It only accepts the certificate with the pinned
/// fingerprint, sends every request exactly once and follows no redirects.
#[derive(Debug, Clone)]
pub struct NasClient {
    base: Url,
    pin: Fingerprint,
    seen: Seen,
    http: reqwest::Client,
}

impl NasClient {
    /// Client for `base_url` (must be `https://…`) with the default limits.
    pub fn new(base_url: &str, pin: Fingerprint) -> Result<Self, ProviderError> {
        Self::with_timeouts(base_url, pin, TIMEOUT, CONNECT_TIMEOUT)
    }

    /// Like [`NasClient::new`] with own limits, for tests.
    pub fn with_timeouts(
        base_url: &str,
        pin: Fingerprint,
        total: Duration,
        connect: Duration,
    ) -> Result<Self, ProviderError> {
        let base = parse_base(base_url)?;
        let seen: Seen = Arc::new(Mutex::new(None));
        let tls = verifier::pinned_config(pin, seen.clone())
            .map_err(|e| ProviderError::Parse(format!("TLS-Konfiguration: {e}")))?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            // HTTPS_PROXY & Co. would put a proxy between app and NAS; its
            // certificate would fail the pin.
            .no_proxy()
            .https_only(true)
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .timeout(total)
            .connect_timeout(connect)
            .user_agent(USER_AGENT)
            // Cookies stay with this client, so with this one NAS. The
            // Container Station keeps its session in a cookie.
            .cookie_store(true)
            .build()
            .map_err(|e| ProviderError::Parse(format!("HTTP-Client: {}", chain(&e))))?;
        Ok(Self {
            base,
            pin,
            seen,
            http,
        })
    }

    /// The pinned fingerprint.
    pub fn pin(&self) -> Fingerprint {
        self.pin
    }

    /// `GET <base><path>?<query>`, body as text.
    pub async fn get_text(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<String, ProviderError> {
        let url = self.url(path)?;
        self.send(self.http.get(url).query(query)).await
    }

    /// Like [`NasClient::get_text`] with `Authorization: Bearer <token>`,
    /// for the collector of NAS Stats (#46). A 401 is
    /// [`ProviderError::LoginFailed`].
    pub async fn get_text_bearer(
        &self,
        path: &str,
        query: &[(&str, &str)],
        token: &str,
    ) -> Result<String, ProviderError> {
        let url = self.url(path)?;
        self.send(self.http.get(url).query(query).bearer_auth(token))
            .await
    }

    /// `POST <base><path>` with a URL-encoded form, body as text.
    pub async fn post_form_text(
        &self,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<String, ProviderError> {
        let url = self.url(path)?;
        self.send(self.http.post(url).form(form)).await
    }

    /// `POST <base><path>` with `body` as JSON (`Content-Type:
    /// application/json`, sent unchanged), body of the answer as text.
    pub async fn post_json_text(&self, path: &str, body: &str) -> Result<String, ProviderError> {
        let url = self.url(path)?;
        self.send(
            self.http
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_owned()),
        )
        .await
    }

    /// Like [`NasClient::post_json_text`] with the header `x-api-key:
    /// <api_key>`, for the GraphQL API of Unraid (#35). A 401 is
    /// [`ProviderError::LoginFailed`].
    pub async fn post_json_text_api_key(
        &self,
        path: &str,
        body: &str,
        api_key: &str,
    ) -> Result<String, ProviderError> {
        let url = self.url(path)?;
        let mut key = reqwest::header::HeaderValue::from_str(api_key)
            .map_err(|_| ProviderError::Parse("API-Key enthält ungültige Zeichen".into()))?;
        key.set_sensitive(true);
        self.send(
            self.http
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header("x-api-key", key)
                .body(body.to_owned()),
        )
        .await
    }

    /// `path` relative to the base URL; it may not lead to another origin.
    fn url(&self, path: &str) -> Result<Url, ProviderError> {
        let url = self
            .base
            .join(path)
            .map_err(|e| ProviderError::Parse(format!("Pfad: {e}")))?;
        if url.origin() != self.base.origin() {
            return Err(ProviderError::Parse(
                "Pfad verlässt die Adresse des NAS".into(),
            ));
        }
        Ok(url)
    }

    async fn send(&self, request: RequestBuilder) -> Result<String, ProviderError> {
        let response = request
            .send()
            .await
            .map_err(|e| self.map_error(&e.without_url()))?;
        match response.status() {
            s if s.is_success() => {}
            StatusCode::UNAUTHORIZED => return Err(ProviderError::LoginFailed),
            StatusCode::FORBIDDEN => return Err(ProviderError::PermissionDenied),
            s => return Err(ProviderError::Http(s.as_u16())),
        }
        response.text().await.map_err(|e| {
            let e = e.without_url();
            if e.is_timeout() {
                ProviderError::Timeout
            } else {
                ProviderError::Parse(chain(&e))
            }
        })
    }

    fn map_error(&self, e: &reqwest::Error) -> ProviderError {
        // Only a request that failed on the pin itself reports the change.
        // `seen` is shared by all requests of this client and is not cleared,
        // so a value left by another or an earlier handshake cannot turn an
        // unrelated error into `CertificateChanged`.
        if pin_rejected(e) {
            let seen = *self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(new) = seen {
                return ProviderError::CertificateChanged {
                    old: self.pin.to_string(),
                    new: new.to_string(),
                };
            }
        }
        if e.is_timeout() {
            ProviderError::Timeout
        } else if e.is_body() || e.is_decode() {
            ProviderError::Parse(chain(e))
        } else {
            // Connect, request and other TLS errors.
            ProviderError::Unreachable(chain(e))
        }
    }
}

/// `base_url` as `https://host[:port]/…`; anything else is refused.
fn parse_base(base_url: &str) -> Result<Url, ProviderError> {
    let base =
        Url::parse(base_url).map_err(|e| ProviderError::Parse(format!("Adresse des NAS: {e}")))?;
    if base.scheme() != "https" {
        return Err(ProviderError::Parse(
            "Adresse des NAS: nur https:// ist erlaubt".into(),
        ));
    }
    if base.host_str().is_none() {
        return Err(ProviderError::Parse("Adresse des NAS: Host fehlt".into()));
    }
    Ok(base)
}

/// Whether the error comes from the pin verifier rejecting the certificate.
/// The rustls error arrives wrapped in nested `io::Error`s, whose `source()`
/// skips the wrapped error, so those are unwrapped by hand.
fn pin_rejected(e: &reqwest::Error) -> bool {
    let mut source = e.source();
    while let Some(err) = source {
        if is_pin_error(err) {
            return true;
        }
        source = err.source();
    }
    false
}

fn is_pin_error(err: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(rustls_error) = err.downcast_ref::<rustls::Error>() {
        return matches!(
            rustls_error,
            rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure
            )
        );
    }
    match err
        .downcast_ref::<std::io::Error>()
        .and_then(|io| io.get_ref())
    {
        Some(inner) => is_pin_error(inner),
        None => false,
    }
}

/// Reads the server certificate of the NAS at `base_url` (`https://…`, the
/// same address the [`NasClient`] gets later, so both send the same SNI name)
/// and returns its fingerprint. The handshake is aborted right after the
/// certificate, so the server never receives a request.
pub async fn first_contact(base_url: &str) -> Result<Fingerprint, ProviderError> {
    let base = parse_base(base_url)?;
    // `parse_base` made sure there is a host; IPv6 comes in brackets.
    let host = base
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = base.port_or_known_default().unwrap_or(443);
    let name = ServerName::try_from(host.clone())
        .map_err(|e| ProviderError::Parse(format!("Host {host}: {e}")))?;
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| ProviderError::Timeout)?
        .map_err(|e| ProviderError::Unreachable(format!("{host}:{port}: {e}")))?;
    let seen: Seen = Arc::new(Mutex::new(None));
    let config = verifier::capture_config(seen.clone())
        .map_err(|e| ProviderError::Parse(format!("TLS-Konfiguration: {e}")))?;
    let connector = TlsConnector::from(Arc::new(config));
    let handshake = tokio::time::timeout(TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| ProviderError::Timeout)?;
    let captured = seen.lock().unwrap_or_else(PoisonError::into_inner).take();
    match (captured, handshake) {
        (Some(fp), _) => Ok(fp),
        (None, Err(e)) => Err(ProviderError::Unreachable(format!("{host}:{port}: {e}"))),
        // Cannot happen: the capture verifier rejects every certificate.
        (None, Ok(_)) => Err(ProviderError::Unreachable(format!(
            "{host}:{port}: kein Zertifikat erhalten"
        ))),
    }
}

/// The error with all its causes, for the log. Callers strip the URL first
/// (`without_url`): query strings can carry passwords or session ids.
fn chain(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        text.push_str(": ");
        text.push_str(&s.to_string());
        source = s.source();
    }
    text
}
