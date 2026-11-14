//! HTTPS client bound to one NAS and its pinned certificate, and the first
//! contact that reads the certificate before anything is sent.

use std::error::Error as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nas_model::NasError;
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
    pub fn new(base_url: &str, pin: Fingerprint) -> Result<Self, NasError> {
        Self::with_timeouts(base_url, pin, TIMEOUT, CONNECT_TIMEOUT)
    }

    /// Like [`NasClient::new`] with own limits, for tests.
    pub fn with_timeouts(
        base_url: &str,
        pin: Fingerprint,
        total: Duration,
        connect: Duration,
    ) -> Result<Self, NasError> {
        let base = Url::parse(base_url)
            .map_err(|e| NasError::Parse(format!("Adresse {base_url}: {e}")))?;
        if base.scheme() != "https" {
            return Err(NasError::Parse(format!(
                "Adresse {base_url}: nur https:// ist erlaubt"
            )));
        }
        if base.host_str().is_none() {
            return Err(NasError::Parse(format!("Adresse {base_url}: Host fehlt")));
        }
        let seen: Seen = Arc::new(Mutex::new(None));
        let tls = verifier::pinned_config(pin, seen.clone())
            .map_err(|e| NasError::Parse(format!("TLS-Konfiguration: {e}")))?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(true)
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .timeout(total)
            .connect_timeout(connect)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| NasError::Parse(format!("HTTP-Client: {}", chain(&e))))?;
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
    pub async fn get_text(&self, path: &str, query: &[(&str, &str)]) -> Result<String, NasError> {
        let url = self.url(path)?;
        self.send(self.http.get(url).query(query)).await
    }

    /// `POST <base><path>` with a URL-encoded form, body as text.
    pub async fn post_form_text(
        &self,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<String, NasError> {
        let url = self.url(path)?;
        self.send(self.http.post(url).form(form)).await
    }

    /// `path` relative to the base URL; it may not lead to another origin.
    fn url(&self, path: &str) -> Result<Url, NasError> {
        let url = self
            .base
            .join(path)
            .map_err(|e| NasError::Parse(format!("Pfad {path}: {e}")))?;
        if url.origin() != self.base.origin() {
            return Err(NasError::Parse(format!(
                "Pfad {path} verlässt die Adresse des NAS"
            )));
        }
        Ok(url)
    }

    async fn send(&self, request: RequestBuilder) -> Result<String, NasError> {
        let response = request.send().await.map_err(|e| self.map_error(&e))?;
        match response.status() {
            s if s.is_success() => {}
            StatusCode::UNAUTHORIZED => return Err(NasError::LoginFailed),
            StatusCode::FORBIDDEN => return Err(NasError::PermissionDenied),
            s => return Err(NasError::Http(s.as_u16())),
        }
        response.text().await.map_err(|e| {
            if e.is_timeout() {
                NasError::Timeout
            } else {
                NasError::Parse(chain(&e))
            }
        })
    }

    fn map_error(&self, e: &reqwest::Error) -> NasError {
        let seen = self
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(new) = seen {
            return NasError::CertificateChanged {
                old: self.pin.to_string(),
                new: new.to_string(),
            };
        }
        if e.is_timeout() {
            NasError::Timeout
        } else if e.is_body() || e.is_decode() {
            NasError::Parse(chain(e))
        } else {
            // Connect, request and other TLS errors.
            NasError::Unreachable(chain(e))
        }
    }
}

/// Reads the server certificate of `host:port` and returns its fingerprint.
/// The handshake is aborted right after the certificate, so the server never
/// receives a request. `host` may be a name or an IP address.
pub async fn first_contact(host: &str, port: u16) -> Result<Fingerprint, NasError> {
    let name = ServerName::try_from(host.to_owned())
        .map_err(|e| NasError::Parse(format!("Host {host}: {e}")))?;
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| NasError::Timeout)?
        .map_err(|e| NasError::Unreachable(format!("{host}:{port}: {e}")))?;
    let seen: Seen = Arc::new(Mutex::new(None));
    let config = verifier::capture_config(seen.clone())
        .map_err(|e| NasError::Parse(format!("TLS-Konfiguration: {e}")))?;
    let connector = TlsConnector::from(Arc::new(config));
    let handshake = tokio::time::timeout(TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| NasError::Timeout)?;
    let captured = seen.lock().unwrap_or_else(PoisonError::into_inner).take();
    match (captured, handshake) {
        (Some(fp), _) => Ok(fp),
        (None, Err(e)) => Err(NasError::Unreachable(format!("{host}:{port}: {e}"))),
        // Cannot happen: the capture verifier rejects every certificate.
        (None, Ok(_)) => Err(NasError::Unreachable(format!(
            "{host}:{port}: kein Zertifikat erhalten"
        ))),
    }
}

/// The error with all its causes, for the log.
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
