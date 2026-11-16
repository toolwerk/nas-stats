//! The interface for the apps: HTTPS with the certificate of the pairing,
//! HTTP/1.1, every request with `Authorization: Bearer <key>`. Read only:
//!
//! - `GET /v1/nas`: the configured NAS with their state
//!   ([`stats_core::CollectorNas`]).
//! - `GET /v1/export?nas=<id>&since=<unix seconds>`: the history of one NAS
//!   ([`stats_core::Export`]); `since` is optional.
//!
//! Without a valid key every path answers 401. An unknown NAS or path is
//! 404, a `since` that is no number 400, another method than GET 405.
//! Handshake and request head each have 10 seconds.

use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue, WWW_AUTHENTICATE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use sha2::{Digest, Sha256};
use stats_core::{Clock, CollectorNas, HistoryStore, Planner};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;

use crate::config::NasConfig;
use crate::pairing::Pairing;

/// Time limit for the TLS handshake and for the request head.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the interface reads.
pub struct Api {
    /// Configured NAS, in the order of the file.
    nas: Vec<NasConfig>,
    planner: Arc<Planner>,
    /// A connection of its own to the archive, only for reading.
    store: Mutex<HistoryStore>,
    /// SHA-256 of the key; compared as hash so the time of the comparison
    /// tells nothing about the key.
    token_hash: [u8; 32],
    clock: Clock,
}

impl Api {
    pub fn new(
        nas: Vec<NasConfig>,
        planner: Arc<Planner>,
        store: HistoryStore,
        token: &str,
        clock: Clock,
    ) -> Self {
        Self {
            nas,
            planner,
            store: Mutex::new(store),
            token_hash: Sha256::digest(token.as_bytes()).into(),
            clock,
        }
    }

    fn authorized(&self, header: Option<&HeaderValue>) -> bool {
        let Some(token) = header
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        else {
            return false;
        };
        let hash: [u8; 32] = Sha256::digest(token.trim().as_bytes()).into();
        hash == self.token_hash
    }

    /// Status and JSON body for a request.
    fn answer(
        &self,
        method: &Method,
        path: &str,
        query: Option<&str>,
        auth: Option<&HeaderValue>,
    ) -> (StatusCode, String) {
        if !self.authorized(auth) {
            return error(StatusCode::UNAUTHORIZED, "Schlüssel fehlt oder ist falsch");
        }
        let known = matches!(path, "/v1/nas" | "/v1/export");
        if !known {
            return error(StatusCode::NOT_FOUND, "unbekannter Pfad");
        }
        if method != Method::GET {
            return error(StatusCode::METHOD_NOT_ALLOWED, "nur GET");
        }
        let result = match path {
            "/v1/nas" => self.list(),
            _ => self.export(query.unwrap_or("")),
        };
        result.unwrap_or_else(|(status, text)| error(status, &text))
    }

    fn list(&self) -> Result<(StatusCode, String), (StatusCode, String)> {
        let store = self.lock_store();
        let mut list = Vec::new();
        for nas in &self.nas {
            let state = self.planner.state(&nas.id);
            let reported = state
                .as_ref()
                .and_then(|state| state.snapshot.as_ref())
                .map(|snapshot| &snapshot.nas);
            list.push(CollectorNas {
                id: nas.id.clone(),
                provider: nas.provider,
                name: reported.and_then(|n| n.name.clone()),
                model: reported.and_then(|n| n.model.clone()),
                base_url: nas.base_url.clone(),
                fingerprint: nas.fingerprint.to_string(),
                last_seen: store.last_seen(&nas.id).map_err(internal)?,
                blocked: state.as_ref().is_some_and(|state| state.blocked),
                error: state
                    .as_ref()
                    .and_then(|state| state.error.as_ref())
                    .map(ToString::to_string),
            });
        }
        json(&list)
    }

    fn export(&self, query: &str) -> Result<(StatusCode, String), (StatusCode, String)> {
        let mut nas_id = None;
        let mut since = None;
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key {
                "nas" => nas_id = Some(value),
                "since" => {
                    let parsed: i64 = value.parse().map_err(|_| {
                        (
                            StatusCode::BAD_REQUEST,
                            format!("since ist keine Zahl: {value:?}"),
                        )
                    })?;
                    since = Some(parsed);
                }
                _ => {}
            }
        }
        let Some(nas_id) = nas_id else {
            return Err((StatusCode::BAD_REQUEST, "nas fehlt".into()));
        };
        if !self.nas.iter().any(|nas| nas.id == nas_id) {
            return Err((StatusCode::NOT_FOUND, format!("kein NAS {nas_id:?}")));
        }
        let export = self
            .lock_store()
            .export(nas_id, since, (self.clock)())
            .map_err(internal)?;
        json(&export)
    }

    fn lock_store(&self) -> std::sync::MutexGuard<'_, HistoryStore> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn json<T: serde::Serialize>(value: &T) -> Result<(StatusCode, String), (StatusCode, String)> {
    serde_json::to_string(value)
        .map(|body| (StatusCode::OK, body))
        .map_err(internal)
}

fn error(status: StatusCode, text: &str) -> (StatusCode, String) {
    let body = serde_json::json!({ "error": text }).to_string();
    (status, body)
}

async fn handle(api: Arc<Api>, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    let auth = request.headers().get(AUTHORIZATION).cloned();
    // The archive is SQLite: read it off the async threads.
    let answer = tokio::task::spawn_blocking(move || {
        api.answer(&method, &path, query.as_deref(), auth.as_ref())
    })
    .await
    .unwrap_or_else(internal);
    let (status, body) = answer;
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if status == StatusCode::UNAUTHORIZED {
        headers.insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

/// TLS settings of the interface, with the certificate of the pairing.
pub fn acceptor(pairing: &Pairing) -> Result<TlsAcceptor, rustls::Error> {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(vec![pairing.cert.clone()], pairing.key.clone_key())?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Answers requests on `listener` until `stop` is notified.
pub async fn serve(listener: TcpListener, tls: TlsAcceptor, api: Arc<Api>, stop: Arc<Notify>) {
    loop {
        let (tcp, _) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                // E.g. too many open files: try again shortly.
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            () = stop.notified() => return,
        };
        let tls = tls.clone();
        let api = Arc::clone(&api);
        tokio::spawn(async move {
            let Ok(Ok(stream)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, tls.accept(tcp)).await
            else {
                return;
            };
            let service = service_fn(move |request| {
                let api = Arc::clone(&api);
                async move { Ok::<_, Infallible>(handle(api, request).await) }
            });
            let _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HANDSHAKE_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_http::{NasClient, ProviderError};
    use nas_model::{BoxFuture, Provider, ProviderKind, Snapshot};
    use stats_core::{Export, PollSink, PollState};

    const T0: i64 = 1_800_000_000;

    struct NoSink;
    impl PollSink for NoSink {
        fn polled(&self, _state: &PollState) {}
    }

    struct Never;
    impl Provider for Never {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Qnap
        }
        fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
            Box::pin(std::future::pending())
        }
    }

    fn nas(id: &str) -> NasConfig {
        NasConfig {
            id: id.into(),
            provider: ProviderKind::Qnap,
            base_url: "https://192.0.2.10".into(),
            fingerprint: nas_http::Fingerprint::of_der(b"nas"),
            user: "nasstats".into(),
            password: "geheim".into(),
        }
    }

    struct Running {
        url: String,
        pin: nas_http::Fingerprint,
        token: String,
        stop: Arc<Notify>,
    }

    impl Running {
        fn client(&self) -> NasClient {
            NasClient::new(&self.url, self.pin).expect("client")
        }
    }

    async fn start() -> Running {
        let dir = std::env::temp_dir().join(format!(
            "nas-collector-server-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let pairing = crate::pairing::ensure(&dir).expect("pairing");
        let clock: Clock = Arc::new(|| T0);
        let planner = Planner::new(300, Arc::clone(&clock), Arc::new(NoSink));
        planner.add("keller", Arc::new(Never), Some(T0 - 60), true);
        let mut store = HistoryStore::in_memory().expect("store");
        let mut export = Export {
            version: stats_core::EXPORT_VERSION,
            nas_id: "keller".into(),
            until: T0,
            samples: vec![stats_core::ExportSample {
                metric: "cpu".into(),
                key: String::new(),
                at: T0 - 600,
                value: 7.0,
            }],
            hourly: Vec::new(),
            daily: Vec::new(),
            backup_runs: Vec::new(),
        };
        store.import(&export, "keller", T0).expect("history");
        export.samples[0].at = T0 - 300;
        store.import(&export, "keller", T0).expect("history");
        let api = Arc::new(Api::new(
            vec![nas("keller"), nas("buero")],
            planner,
            store,
            &pairing.token,
            clock,
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let stop = Arc::new(Notify::new());
        tokio::spawn(serve(
            listener,
            acceptor(&pairing).expect("tls"),
            api,
            Arc::clone(&stop),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Running {
            url: format!("https://127.0.0.1:{port}/"),
            pin: pairing.fingerprint(),
            token: pairing.token,
            stop,
        }
    }

    #[tokio::test]
    async fn without_the_key_every_path_is_401() {
        let server = start().await;
        let client = server.client();
        for path in ["v1/nas", "v1/export?nas=keller", "nirgends"] {
            assert_eq!(
                client.get_text(path, &[]).await,
                Err(ProviderError::LoginFailed),
                "{path}"
            );
            assert_eq!(
                client.get_text_bearer(path, &[], "falsch").await,
                Err(ProviderError::LoginFailed),
                "{path}"
            );
        }
        server.stop.notify_one();
    }

    #[tokio::test]
    async fn with_the_key_the_list_and_the_export_come() {
        let server = start().await;
        let client = server.client();
        let body = client
            .get_text_bearer("v1/nas", &[], &server.token)
            .await
            .expect("list");
        let list: Vec<CollectorNas> = serde_json::from_str(&body).expect("list parses");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "keller");
        assert_eq!(list[0].last_seen, None);
        assert!(list[0].blocked);
        assert_eq!(list[0].fingerprint, nas("keller").fingerprint.to_string());
        assert!(!body.contains("geheim"));
        assert!(!body.contains("nasstats"));
        assert_eq!(list[1].id, "buero");
        assert!(!list[1].blocked);

        let body = client
            .get_text_bearer("v1/export", &[("nas", "keller")], &server.token)
            .await
            .expect("export");
        let export: Export = serde_json::from_str(&body).expect("export parses");
        assert_eq!(export.until, T0);
        assert_eq!(export.samples.len(), 2);

        let body = client
            .get_text_bearer(
                "v1/export",
                &[("nas", "keller"), ("since", &(T0 - 400).to_string())],
                &server.token,
            )
            .await
            .expect("export since");
        let export: Export = serde_json::from_str(&body).expect("export parses");
        assert_eq!(export.samples.len(), 1);
        server.stop.notify_one();
    }

    #[tokio::test]
    async fn wrong_requests_get_their_status() {
        let server = start().await;
        let client = server.client();
        let token = server.token.as_str();
        for (path, query, status) in [
            ("v1/export", vec![("nas", "garage")], 404),
            (
                "v1/export",
                vec![("nas", "keller"), ("since", "gestern")],
                400,
            ),
            ("v1/export", vec![], 400),
            ("v2/nas", vec![], 404),
        ] {
            assert_eq!(
                client.get_text_bearer(path, &query, token).await,
                Err(ProviderError::Http(status)),
                "{path} {query:?}"
            );
        }
        // NasClient posts without a key: 401 comes before 405.
        assert_eq!(
            client.post_form_text("v1/nas", &[]).await,
            Err(ProviderError::LoginFailed)
        );
        server.stop.notify_one();
    }

    #[test]
    fn other_methods_are_405() {
        let clock: Clock = Arc::new(|| T0);
        let planner = Planner::new(300, Arc::clone(&clock), Arc::new(NoSink));
        let api = Api::new(
            vec![nas("keller")],
            planner,
            HistoryStore::in_memory().expect("store"),
            "abc",
            clock,
        );
        let auth = HeaderValue::from_static("Bearer abc");
        let (status, _) = api.answer(&Method::POST, "/v1/nas", None, Some(&auth));
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        let (status, _) = api.answer(&Method::GET, "/v1/nas", None, Some(&auth));
        assert_eq!(status, StatusCode::OK);
        let (status, _) = api.answer(&Method::GET, "/v1/nas", None, None);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
