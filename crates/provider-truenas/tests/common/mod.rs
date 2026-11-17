//! A fake TrueNAS: TLS with a fresh self-signed certificate, WebSocket on
//! `/api/current` only, JSON-RPC 2.0 with `auth.mechanism_choices`,
//! `auth.login_ex` (plain key and SCRAM-SHA-512 with channel binding, built
//! here with ring on its own, not with the code under test), `system.info`
//! and a few test methods. It logs every message it receives.
// `result_large_err`: the callback type of tungstenite returns its response by value.
#![allow(clippy::unwrap_used, clippy::result_large_err, dead_code)]

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use futures_util::{SinkExt, StreamExt};
use nas_http::Fingerprint;
use provider_truenas::TruenasCredentials;
use ring::{digest, hmac, pbkdf2};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

pub const USER: &str = "nasstats";
/// A made-up key in the form TrueNAS shows.
pub const API_KEY: &str = "7-Kx9Lm2Pq4Rs6Tv8Wy0ZaBc3De5Fg";
const KEY_ID: &str = "7";
const SECRET: &str = "Kx9Lm2Pq4Rs6Tv8Wy0ZaBc3De5Fg";
const SALT: [u8; 16] = *b"salz-des-truenas";
const ITERATIONS: u32 = 50_000;

/// Which TrueNAS the server plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Version {
    /// 25.x: `auth.mechanism_choices` fails, only `API_KEY_PLAIN`.
    V25,
    /// 26: offers SCRAM and insists on the channel binding.
    V26,
}

/// How the login ends, whatever the client sends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Login {
    /// Checks user and key as TrueNAS would.
    Check,
    AuthErr,
    OtpRequired,
    Redirect,
    /// SCRAM succeeds on the server, but its signature is wrong.
    BadSignature,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub version: Version,
    pub login: Login,
    /// Refuse the WebSocket upgrade with this status.
    pub upgrade_status: Option<u16>,
}

impl Config {
    pub fn new(version: Version) -> Self {
        Self {
            version,
            login: Login::Check,
            upgrade_status: None,
        }
    }
}

pub struct FakeTruenas {
    addr: SocketAddr,
    cert: CertificateDer<'static>,
    log: Arc<Mutex<Vec<String>>>,
    connections: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeTruenas {
    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}/", self.addr.port())
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_der(&self.cert)
    }

    /// Every message the server received, in order.
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Methods of the received calls, in order.
    pub fn methods(&self) -> Vec<String> {
        self.log()
            .iter()
            .filter_map(|m| serde_json::from_str::<Value>(m).ok())
            .filter_map(|v| v.get("method").and_then(Value::as_str).map(str::to_owned))
            .collect()
    }

    /// TCP connections accepted so far.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for FakeTruenas {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn credentials() -> TruenasCredentials {
    TruenasCredentials::new(USER, API_KEY).unwrap()
}

pub async fn start(config: Config) -> FakeTruenas {
    let key = rcgen::generate_simple_self_signed(vec!["truenas.example".to_owned()]).unwrap();
    let cert = key.cert.der().clone();
    let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.signing_key.serialize_der()));
    let tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], private)
            .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let keys = Arc::new(ServerKeys::new());
    let task = {
        let log = log.clone();
        let connections = connections.clone();
        let cert = cert.to_vec();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                connections.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let connection = Connection {
                    config,
                    log: log.clone(),
                    keys: keys.clone(),
                    cert: cert.clone(),
                    authenticated: false,
                    scram: None,
                };
                tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(tcp).await {
                        connection.serve(tls).await;
                    }
                });
            }
        })
    };
    FakeTruenas {
        addr,
        cert,
        log,
        connections,
        task,
    }
}

/// What the server stores for the key: StoredKey and ServerKey.
struct ServerKeys {
    stored_key: Vec<u8>,
    server_key: Vec<u8>,
}

impl ServerKeys {
    fn new() -> Self {
        let mut salted = [0u8; 64];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA512,
            NonZeroU32::new(ITERATIONS).unwrap(),
            &SALT,
            SECRET.as_bytes(),
            &mut salted,
        );
        let key = hmac::Key::new(hmac::HMAC_SHA512, &salted);
        let client_key = hmac::sign(&key, b"Client Key");
        Self {
            stored_key: digest::digest(&digest::SHA512, client_key.as_ref())
                .as_ref()
                .to_vec(),
            server_key: hmac::sign(&key, b"Server Key").as_ref().to_vec(),
        }
    }
}

/// SCRAM state between the two messages of the client.
struct Scram {
    client_first_bare: String,
    server_first: String,
    nonce: String,
}

struct Connection {
    config: Config,
    log: Arc<Mutex<Vec<String>>>,
    keys: Arc<ServerKeys>,
    cert: Vec<u8>,
    authenticated: bool,
    scram: Option<Scram>,
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>;

/// What to do with a call.
enum Reply {
    Result(Value),
    Error(Value),
    /// Send these first, then the result.
    Noise(Vec<Value>, Value),
    Silent,
    Close,
}

impl Connection {
    async fn serve(mut self, tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>) {
        let refuse = self.config.upgrade_status;
        let callback = move |request: &Request, response: Response| {
            let status = match refuse {
                Some(status) => Some(status),
                None if request.uri().path() != "/api/current" => Some(404),
                None => None,
            };
            match status {
                Some(status) => {
                    let mut error = ErrorResponse::new(Some("nein".to_owned()));
                    *error.status_mut() = StatusCode::from_u16(status).unwrap();
                    Err(error)
                }
                None => Ok(response),
            }
        };
        let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tls, callback).await else {
            return;
        };
        while let Some(Ok(message)) = ws.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            self.log.lock().unwrap().push(text.to_string());
            let call: Value = serde_json::from_str(&text).unwrap();
            let id = call["id"].clone();
            let reply = self.answer(&call["method"], &call["params"]);
            let done = send(&mut ws, &id, reply).await;
            if done {
                return;
            }
        }
    }

    fn answer(&mut self, method: &Value, params: &Value) -> Reply {
        match method.as_str().unwrap_or_default() {
            "auth.mechanism_choices" => match self.config.version {
                Version::V25 => Reply::Error(json!({
                    "code": -32001,
                    "message": "Method call error",
                    "data": {"reason": "'NoneType' object has no attribute 'may_create_auth_token'"},
                })),
                Version::V26 => Reply::Result(json!(["PASSWORD_PLAIN", "API_KEY_PLAIN", "SCRAM"])),
            },
            "auth.login_ex" => Reply::Result(self.login(&params[0])),
            "system.info" if self.authenticated => {
                Reply::Result(json!({"hostname": "truenas-demo", "version": "TrueNAS-26.04.0"}))
            }
            "system.info" => Reply::Error(json!({
                "code": -32001,
                "message": "Method call error",
                "data": {"errname": "ENOTAUTHENTICATED", "reason": "Not authenticated"},
            })),
            "test.noise" => Reply::Noise(
                vec![
                    json!({"jsonrpc": "2.0", "method": "collection_update", "params": {"msg": "added"}}),
                    json!({"jsonrpc": "2.0", "id": 999_999, "result": "fremd"}),
                ],
                json!("eigene"),
            ),
            "test.eacces" => Reply::Error(json!({
                "code": -32001,
                "message": "Method call error",
                "data": {"error": 13, "errname": "EACCES", "reason": "Not authorized"},
            })),
            "test.silent" => Reply::Silent,
            "test.close" => Reply::Close,
            _ => Reply::Error(json!({"code": -32601, "message": "Method does not exist"})),
        }
    }

    fn login(&mut self, params: &Value) -> Value {
        match self.config.login {
            Login::AuthErr => return json!({"response_type": "AUTH_ERR"}),
            Login::OtpRequired => return json!({"response_type": "OTP_REQUIRED"}),
            Login::Redirect => {
                return json!({"response_type": "REDIRECT", "urls": ["wss://192.0.2.2/api/current"]});
            }
            Login::Check | Login::BadSignature => {}
        }
        match params["mechanism"].as_str() {
            Some("API_KEY_PLAIN") if self.config.version == Version::V25 => {
                if params["username"] == USER && params["api_key"] == API_KEY {
                    self.authenticated = true;
                    json!({"response_type": "SUCCESS", "user_info": null})
                } else {
                    json!({"response_type": "AUTH_ERR"})
                }
            }
            Some("SCRAM") if self.config.version == Version::V26 => {
                let rfc = params["rfc_str"].as_str().unwrap_or_default();
                let reply = match params["scram_type"].as_str() {
                    Some("CLIENT_FIRST_MESSAGE") => self.scram_first(rfc),
                    Some("CLIENT_FINAL_MESSAGE") => self.scram_final(rfc),
                    _ => None,
                };
                reply.unwrap_or_else(|| json!({"response_type": "AUTH_ERR"}))
            }
            _ => json!({"response_type": "AUTH_ERR"}),
        }
    }

    fn scram_first(&mut self, rfc: &str) -> Option<Value> {
        let bare = rfc.strip_prefix("p=tls-server-end-point,,")?;
        let rest = bare.strip_prefix(&format!("n={USER}:{KEY_ID},r="))?;
        let client_nonce = B64.decode(rest).ok()?;
        let nonce =
            B64.encode([client_nonce, b"server-nonce-0123456789abcdefghi".to_vec()].concat());
        let server_first = format!("r={nonce},s={},i={ITERATIONS}", B64.encode(SALT));
        self.scram = Some(Scram {
            client_first_bare: bare.to_owned(),
            server_first: server_first.clone(),
            nonce,
        });
        Some(json!({
            "response_type": "SCRAM_RESPONSE",
            "scram_type": "SERVER_FIRST_RESPONSE",
            "rfc_str": server_first,
        }))
    }

    fn scram_final(&mut self, rfc: &str) -> Option<Value> {
        let scram = self.scram.take()?;
        let (without_proof, proof) = rfc.rsplit_once(",p=")?;
        // The binding the server expects: SHA-256 of its own ECDSA-SHA256
        // certificate behind the GS2 header.
        let mut cbind = b"p=tls-server-end-point,,".to_vec();
        cbind.extend_from_slice(digest::digest(&digest::SHA256, &self.cert).as_ref());
        if without_proof != format!("c={},r={}", B64.encode(cbind), scram.nonce) {
            return None;
        }
        let auth_message = format!(
            "{},{},{without_proof}",
            scram.client_first_bare, scram.server_first
        );
        let stored = hmac::Key::new(hmac::HMAC_SHA512, &self.keys.stored_key);
        let signature = hmac::sign(&stored, auth_message.as_bytes());
        let proof = B64.decode(proof).ok()?;
        if proof.len() != 64 {
            return None;
        }
        let client_key: Vec<u8> = proof
            .iter()
            .zip(signature.as_ref())
            .map(|(p, s)| p ^ s)
            .collect();
        if digest::digest(&digest::SHA512, &client_key).as_ref() != self.keys.stored_key {
            return None;
        }
        self.authenticated = true;
        let server = hmac::Key::new(hmac::HMAC_SHA512, &self.keys.server_key);
        let mut server_signature = hmac::sign(&server, auth_message.as_bytes())
            .as_ref()
            .to_vec();
        if self.config.login == Login::BadSignature {
            server_signature[0] ^= 1;
        }
        Some(json!({
            "response_type": "SCRAM_RESPONSE",
            "scram_type": "SERVER_FINAL_RESPONSE",
            "rfc_str": format!("v={}", B64.encode(server_signature)),
        }))
    }
}

/// Sends the reply; true when the connection is to end.
async fn send(ws: &mut Ws, id: &Value, reply: Reply) -> bool {
    let message = |v: Value| Message::text(v.to_string());
    match reply {
        Reply::Result(result) => {
            let _ = ws
                .send(message(
                    json!({"jsonrpc": "2.0", "id": id, "result": result}),
                ))
                .await;
        }
        Reply::Error(error) => {
            let _ = ws
                .send(message(json!({"jsonrpc": "2.0", "id": id, "error": error})))
                .await;
        }
        Reply::Noise(noise, result) => {
            for n in noise {
                let _ = ws.send(message(n)).await;
            }
            let _ = ws
                .send(message(
                    json!({"jsonrpc": "2.0", "id": id, "result": result}),
                ))
                .await;
        }
        Reply::Silent => {}
        Reply::Close => {
            let _ = ws.close(None).await;
            return true;
        }
    }
    false
}
