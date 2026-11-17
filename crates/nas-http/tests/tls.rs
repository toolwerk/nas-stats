//! The HTTPS layer against a local TLS server with a self-signed certificate.
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nas_http::{Fingerprint, NasClient, ProviderError, first_contact};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// What the server does once it has read a request.
#[derive(Clone, Copy)]
enum Answer {
    Status(u16),
    /// 200 with `Set-Cookie: CSID=abc`.
    Cookie,
    Never,
}

struct Server {
    addr: SocketAddr,
    cert: CertificateDer<'static>,
    requests: Arc<AtomicUsize>,
    /// Request line and headers of every request, in arrival order.
    heads: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    fn url(&self) -> String {
        format!("https://127.0.0.1:{}/", self.addr.port())
    }

    fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_der(&self.cert)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }

    /// Stops accepting; the port is closed afterwards.
    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

/// Starts a TLS server on `127.0.0.1:0` with a fresh self-signed certificate
/// for `localhost`. It counts every HTTP request it reads.
async fn server(answer: Answer) -> Server {
    let key = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert = key.cert.der().clone();
    let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.signing_key.serialize_der()));
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], private)
            .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = heads.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let counter = counter.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let end = head.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head[..end]).into_owned());
                counter.fetch_add(1, Ordering::SeqCst);
                match answer {
                    Answer::Never => tokio::time::sleep(Duration::from_secs(30)).await,
                    Answer::Status(_) | Answer::Cookie => {
                        let (code, cookie) = match answer {
                            Answer::Status(code) => (code, ""),
                            _ => (200, "Set-Cookie: CSID=abc; Path=/\r\n"),
                        };
                        let body = "hallo";
                        let response = format!(
                            "HTTP/1.1 {code} X\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = tls.write_all(response.as_bytes()).await;
                        let _ = tls.shutdown().await;
                    }
                }
            });
        }
    });
    Server {
        addr,
        cert,
        requests,
        heads,
        task,
    }
}

/// Value of header `name` in a request head, case-insensitive name.
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[tokio::test]
async fn first_contact_returns_fingerprint_without_request() {
    let server = server(Answer::Status(200)).await;
    let fp = first_contact(&server.url()).await.unwrap();
    assert_eq!(fp, server.fingerprint());
    // Give the server task the chance to read a request, if one came.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.requests(), 0);
}

#[tokio::test]
async fn matching_pin_gets_the_body() {
    let server = server(Answer::Status(200)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let body = client
        .get_text("cgi-bin/x.cgi", &[("a", "1")])
        .await
        .unwrap();
    assert_eq!(body, "hallo");
    let body = client
        .post_form_text("login", &[("user", "u")])
        .await
        .unwrap();
    assert_eq!(body, "hallo");
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn other_certificate_is_certificate_changed() {
    let server = server(Answer::Status(200)).await;
    let pin = Fingerprint::of_der(b"another certificate");
    let client = NasClient::new(&server.url(), pin).unwrap();
    let err = client.get_text("", &[]).await.unwrap_err();
    assert_eq!(
        err,
        ProviderError::CertificateChanged {
            old: pin.to_string(),
            new: server.fingerprint().to_string(),
        }
    );
    assert_eq!(server.requests(), 0);
}

#[tokio::test]
async fn plain_http_is_rejected() {
    let pin = Fingerprint::of_der(b"cert");
    let err = NasClient::new("http://127.0.0.1:8080/", pin).unwrap_err();
    assert!(matches!(err, ProviderError::Parse(_)), "{err:?}");
}

#[tokio::test]
async fn path_cannot_leave_the_nas() {
    let server = server(Answer::Status(200)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    for path in [
        "http://127.0.0.1/",
        "https://example.com/",
        "//example.com/x",
    ] {
        let err = client.get_text(path, &[]).await.unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)), "{path}: {err:?}");
    }
    assert_eq!(server.requests(), 0);
}

#[tokio::test]
async fn status_codes_map_to_errors_with_one_request_each() {
    for (code, expected) in [
        (401, ProviderError::LoginFailed),
        (403, ProviderError::PermissionDenied),
        (500, ProviderError::Http(500)),
        (302, ProviderError::Http(302)),
    ] {
        let server = server(Answer::Status(code)).await;
        let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
        let err = client.get_text("", &[]).await.unwrap_err();
        assert_eq!(err, expected, "status {code}");
        assert_eq!(server.requests(), 1, "status {code}");
    }
}

#[tokio::test]
async fn silent_server_is_timeout() {
    let server = server(Answer::Never).await;
    let client = NasClient::with_timeouts(
        &server.url(),
        server.fingerprint(),
        Duration::from_millis(300),
        Duration::from_millis(300),
    )
    .unwrap();
    let err = client.get_text("", &[]).await.unwrap_err();
    assert_eq!(err, ProviderError::Timeout);
    assert_eq!(server.requests(), 1);
}

#[tokio::test]
async fn closed_port_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = NasClient::new(
        &format!("https://127.0.0.1:{port}/"),
        Fingerprint::of_der(b"cert"),
    )
    .unwrap();
    let err = client
        .get_text("cgi-bin/login.cgi", &[("pwd", "geheim")])
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Unreachable(_)), "{err:?}");
    // The detail goes to the log; it must not carry the query string.
    assert!(!format!("{err:?}").contains("geheim"), "{err:?}");
    let err = first_contact(&format!("https://127.0.0.1:{port}/"))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Unreachable(_)), "{err:?}");
}

#[tokio::test]
async fn concurrent_requests_all_see_the_changed_certificate() {
    let server = server(Answer::Status(200)).await;
    let pin = Fingerprint::of_der(b"another certificate");
    let client = NasClient::new(&server.url(), pin).unwrap();
    let (a, b, c) = tokio::join!(
        client.get_text("a", &[]),
        client.get_text("b", &[]),
        client.get_text("c", &[]),
    );
    for err in [a, b, c] {
        assert!(
            matches!(err, Err(ProviderError::CertificateChanged { .. })),
            "{err:?}"
        );
    }
}

#[tokio::test]
async fn later_unrelated_error_is_not_certificate_changed() {
    // A pin failure leaves its fingerprint behind in the client; a later
    // error of another kind must keep its own variant.
    let server = server(Answer::Status(200)).await;
    let client = NasClient::new(&server.url(), Fingerprint::of_der(b"other")).unwrap();
    assert!(matches!(
        client.get_text("", &[]).await,
        Err(ProviderError::CertificateChanged { .. })
    ));
    server.stop().await;
    let err = client.get_text("", &[]).await.unwrap_err();
    assert!(matches!(err, ProviderError::Unreachable(_)), "{err:?}");
}

#[tokio::test]
async fn cookies_are_kept_and_json_is_posted_as_json() {
    let server = server(Answer::Cookie).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let body = client
        .post_json_text("/container-station/api/v1/login", r#"{"username":"u"}"#)
        .await
        .unwrap();
    assert_eq!(body, "hallo");
    client
        .get_text("/container-station/api/v1/container", &[])
        .await
        .unwrap();
    let heads = server.heads();
    assert_eq!(heads.len(), 2);
    assert!(
        heads[0].starts_with("POST /container-station/api/v1/login "),
        "{}",
        heads[0]
    );
    assert_eq!(header(&heads[0], "content-type"), Some("application/json"));
    assert_eq!(header(&heads[0], "content-length"), Some("16"));
    assert_eq!(header(&heads[0], "cookie"), None);
    assert!(
        heads[1].starts_with("GET /container-station/api/v1/container "),
        "{}",
        heads[1]
    );
    assert_eq!(header(&heads[1], "cookie"), Some("CSID=abc"));
}

#[tokio::test]
async fn bearer_get_sends_the_token() {
    let server = server(Answer::Status(200)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let body = client
        .get_text_bearer("v1/export", &[("nas", "keller")], "abc123")
        .await
        .unwrap();
    assert_eq!(body, "hallo");
    let heads = server.heads();
    assert!(
        heads[0].starts_with("GET /v1/export?nas=keller "),
        "{}",
        heads[0]
    );
    assert_eq!(header(&heads[0], "authorization"), Some("Bearer abc123"));
    // The plain call sends no token.
    client.get_text("v1/nas", &[]).await.unwrap();
    assert_eq!(header(&server.heads()[1], "authorization"), None);
}

#[tokio::test]
async fn api_key_post_sends_the_key() {
    let server = server(Answer::Status(200)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let body = client
        .post_json_text_api_key("graphql", r#"{"query":"{ vars { name } }"}"#, "key123")
        .await
        .unwrap();
    assert_eq!(body, "hallo");
    let heads = server.heads();
    assert!(heads[0].starts_with("POST /graphql "), "{}", heads[0]);
    assert_eq!(header(&heads[0], "x-api-key"), Some("key123"));
    assert_eq!(header(&heads[0], "content-type"), Some("application/json"));
    // The plain call sends no key.
    client.post_json_text("graphql", "{}").await.unwrap();
    assert_eq!(header(&server.heads()[1], "x-api-key"), None);
}

#[tokio::test]
async fn api_key_post_with_rejected_key_is_login_failed() {
    let server = server(Answer::Status(401)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let err = client
        .post_json_text_api_key("graphql", "{}", "falsch")
        .await
        .unwrap_err();
    assert_eq!(err, ProviderError::LoginFailed);
    assert_eq!(server.requests(), 1);
}

#[tokio::test]
async fn api_key_with_a_line_break_is_not_sent() {
    let server = server(Answer::Status(200)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let err = client
        .post_json_text_api_key("graphql", "{}", "key\r\nX-Evil: 1")
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Parse(_)), "{err:?}");
    assert_eq!(server.requests(), 0);
}

#[tokio::test]
async fn bearer_get_with_rejected_token_is_login_failed() {
    let server = server(Answer::Status(401)).await;
    let client = NasClient::new(&server.url(), server.fingerprint()).unwrap();
    let err = client
        .get_text_bearer("v1/nas", &[], "falsch")
        .await
        .unwrap_err();
    assert_eq!(err, ProviderError::LoginFailed);
}
