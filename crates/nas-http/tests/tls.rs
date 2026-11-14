//! The HTTPS layer against a local TLS server with a self-signed certificate.
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nas_http::{Fingerprint, NasClient, NasError, first_contact};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// What the server does once it has read a request.
#[derive(Clone, Copy)]
enum Answer {
    Status(u16),
    Never,
}

struct Server {
    addr: SocketAddr,
    cert: CertificateDer<'static>,
    requests: Arc<AtomicUsize>,
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
    let task = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let counter = counter.clone();
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
                counter.fetch_add(1, Ordering::SeqCst);
                match answer {
                    Answer::Never => tokio::time::sleep(Duration::from_secs(30)).await,
                    Answer::Status(code) => {
                        let body = "hallo";
                        let response = format!(
                            "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        task,
    }
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
        NasError::CertificateChanged {
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
    assert!(matches!(err, NasError::Parse(_)), "{err:?}");
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
        assert!(matches!(err, NasError::Parse(_)), "{path}: {err:?}");
    }
    assert_eq!(server.requests(), 0);
}

#[tokio::test]
async fn status_codes_map_to_errors_with_one_request_each() {
    for (code, expected) in [
        (401, NasError::LoginFailed),
        (403, NasError::PermissionDenied),
        (500, NasError::Http(500)),
        (302, NasError::Http(302)),
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
    assert_eq!(err, NasError::Timeout);
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
    assert!(matches!(err, NasError::Unreachable(_)), "{err:?}");
    // The detail goes to the log; it must not carry the query string.
    assert!(!format!("{err:?}").contains("geheim"), "{err:?}");
    let err = first_contact(&format!("https://127.0.0.1:{port}/"))
        .await
        .unwrap_err();
    assert!(matches!(err, NasError::Unreachable(_)), "{err:?}");
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
            matches!(err, Err(NasError::CertificateChanged { .. })),
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
        Err(NasError::CertificateChanged { .. })
    ));
    server.stop().await;
    let err = client.get_text("", &[]).await.unwrap_err();
    assert!(matches!(err, NasError::Unreachable(_)), "{err:?}");
}
