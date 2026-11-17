//! Login to one TrueNAS: picks SCRAM or the plain key, stops after a
//! rejected login and keeps the derived SCRAM keys.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use nas_http::{Fingerprint, ProviderError};
use serde_json::{Value, json};

use crate::credentials::TruenasCredentials;
use crate::rpc::{Session, endpoint_url};
use crate::scram::{self, ClientFinal, ClientFirst, ScramKeys, ServerFirst};

/// Client for one TrueNAS. Every [`TruenasClient::session`] opens its own
/// connection and logs in.
pub struct TruenasClient {
    base_url: String,
    pin: Fingerprint,
    timeout: Duration,
    state: Mutex<State>,
}

struct State {
    credentials: TruenasCredentials,
    /// Error of the rejected login; no connection until new credentials.
    blocked: Option<ProviderError>,
    /// SCRAM keys of the current credentials per salt and iteration count.
    keys: HashMap<(Vec<u8>, u32), ScramKeys>,
    /// Counts [`TruenasClient::set_credentials`], so a login that started
    /// with the old credentials cannot block the new ones.
    generation: u64,
}

impl std::fmt::Debug for TruenasClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TruenasClient")
            .field("base_url", &self.base_url)
            .field("pin", &self.pin)
            .field("blocked", &self.is_blocked())
            .finish_non_exhaustive()
    }
}

impl TruenasClient {
    /// Client for `base_url` (`https://…`, the address of the web interface;
    /// a trailing `/ui` is dropped)
    /// with the confirmed certificate `pin`. `http://` and `ws://` are
    /// refused before any connection: TrueNAS revokes a key that arrives
    /// without TLS.
    pub fn new(
        base_url: &str,
        pin: Fingerprint,
        credentials: TruenasCredentials,
    ) -> Result<Self, ProviderError> {
        Self::with_timeout(base_url, pin, credentials, nas_http::TIMEOUT)
    }

    /// Like [`TruenasClient::new`] with an own time limit per call, for tests.
    pub fn with_timeout(
        base_url: &str,
        pin: Fingerprint,
        credentials: TruenasCredentials,
        timeout: Duration,
    ) -> Result<Self, ProviderError> {
        endpoint_url(base_url)?;
        Ok(Self {
            base_url: base_url.to_owned(),
            pin,
            timeout,
            state: Mutex::new(State {
                credentials,
                blocked: None,
                keys: HashMap::new(),
                generation: 0,
            }),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a rejected login blocks every further connection.
    pub fn is_blocked(&self) -> bool {
        self.state().blocked.is_some()
    }

    /// New credentials from the user: lifts the block and drops the keys of
    /// the old ones.
    pub fn set_credentials(&self, credentials: TruenasCredentials) {
        let mut state = self.state();
        state.credentials = credentials;
        state.blocked = None;
        state.keys.clear();
        state.generation += 1;
    }

    /// Opens a connection and logs in.
    ///
    /// While blocked, returns the blocking error without a connection. A
    /// rejected key ([`ProviderError::LoginFailed`]) or a required second
    /// factor ([`ProviderError::SecondFactorRequired`]) blocks until
    /// [`TruenasClient::set_credentials`]; other errors do not.
    pub async fn session(&self) -> Result<Session, ProviderError> {
        let (credentials, generation) = {
            let state = self.state();
            if let Some(blocked) = &state.blocked {
                return Err(blocked.clone());
            }
            (state.credentials.clone(), state.generation)
        };
        let mut session = Session::open(&self.base_url, self.pin, self.timeout).await?;
        match self.login(&mut session, &credentials, generation).await {
            Ok(()) => Ok(session),
            Err(error) => {
                session.close().await;
                if matches!(
                    error,
                    ProviderError::LoginFailed | ProviderError::SecondFactorRequired
                ) {
                    let mut state = self.state();
                    if state.generation == generation {
                        state.blocked = Some(error.clone());
                    }
                }
                Err(error)
            }
        }
    }

    async fn login(
        &self,
        session: &mut Session,
        credentials: &TruenasCredentials,
        generation: u64,
    ) -> Result<(), ProviderError> {
        // An error object means a TrueNAS without the method or with the
        // known bug of 25.04.2 to 25.10.2: no SCRAM there. A failed transport
        // ends the login.
        let offers_scram = match session
            .call_rpc("auth.mechanism_choices", json!([]))
            .await?
        {
            Ok(choices) => choices
                .as_array()
                .is_some_and(|c| c.iter().any(|m| m == "SCRAM")),
            Err(_) => false,
        };
        if offers_scram {
            self.login_scram(session, credentials, generation).await
        } else {
            login_plain(session, credentials).await
        }
    }

    async fn login_scram(
        &self,
        session: &mut Session,
        credentials: &TruenasCredentials,
        generation: u64,
    ) -> Result<(), ProviderError> {
        let binding = session.peer_certificate().and_then(scram::channel_binding);
        let first = ClientFirst::new(
            credentials.username(),
            credentials.key_id(),
            scram::random_nonce()?,
            binding,
        );
        log::debug!("TrueNAS: SCRAM, Kanalbindung {}", first.is_bound());
        let answer = scram_step(session, "CLIENT_FIRST_MESSAGE", &first.message()).await?;
        let server_first =
            ServerFirst::parse(scram_answer(&answer, "SERVER_FIRST_RESPONSE")?, &first)?;
        let keys = self
            .keys(
                credentials,
                &server_first.salt,
                server_first.iterations,
                generation,
            )
            .await?;
        let last = ClientFinal::new(&first, &server_first, &keys);
        let answer = scram_step(session, "CLIENT_FINAL_MESSAGE", last.message()).await?;
        last.verify_server(scram_answer(&answer, "SERVER_FINAL_RESPONSE")?, &keys)
    }

    /// The SCRAM keys for `salt` and `iterations`, from memory or derived
    /// off the async threads.
    async fn keys(
        &self,
        credentials: &TruenasCredentials,
        salt: &[u8],
        iterations: NonZeroU32,
        generation: u64,
    ) -> Result<ScramKeys, ProviderError> {
        let slot = (salt.to_vec(), iterations.get());
        if let Some(keys) = self.state().keys.get(&slot) {
            return Ok(keys.clone());
        }
        let secret = credentials.secret().as_bytes().to_vec();
        let salt = salt.to_vec();
        let keys =
            tokio::task::spawn_blocking(move || ScramKeys::derive(&secret, &salt, iterations))
                .await
                .map_err(|e| ProviderError::Parse(format!("SCRAM-Schlüssel: {e}")))?;
        let mut state = self.state();
        if state.generation == generation {
            state.keys.insert(slot, keys.clone());
        }
        Ok(keys)
    }
}

/// `auth.login_ex` with the whole key, for TrueNAS before 26.
async fn login_plain(
    session: &mut Session,
    credentials: &TruenasCredentials,
) -> Result<(), ProviderError> {
    let answer = login_ex(
        session,
        json!({
            "mechanism": "API_KEY_PLAIN",
            "username": credentials.username(),
            "api_key": credentials.api_key(),
            "login_options": {"user_info": false},
        }),
    )
    .await?;
    match response_type(&answer)? {
        "SUCCESS" => Ok(()),
        other => Err(unexpected(other)),
    }
}

/// One `auth.login_ex` step of SCRAM.
async fn scram_step(
    session: &mut Session,
    scram_type: &str,
    rfc_str: &str,
) -> Result<Value, ProviderError> {
    login_ex(
        session,
        json!({"mechanism": "SCRAM", "scram_type": scram_type, "rfc_str": rfc_str}),
    )
    .await
}

/// `auth.login_ex` with `params`. An error object instead of an answer is
/// treated like a rejected login and blocks, so a refused login is never
/// repeated; transport errors pass through.
async fn login_ex(session: &mut Session, params: Value) -> Result<Value, ProviderError> {
    session
        .call_rpc("auth.login_ex", json!([params]))
        .await?
        .map_err(|e| {
            log::warn!(
                "TrueNAS: auth.login_ex mit Fehler {} beantwortet: {}",
                e.code,
                e.message
            );
            ProviderError::LoginFailed
        })
}

/// `rfc_str` of a `SCRAM_RESPONSE` of type `expected`.
fn scram_answer<'a>(answer: &'a Value, expected: &str) -> Result<&'a str, ProviderError> {
    match response_type(answer)? {
        "SCRAM_RESPONSE" => {}
        other => return Err(unexpected(other)),
    }
    if answer.get("scram_type").and_then(Value::as_str) != Some(expected) {
        return Err(ProviderError::Parse(format!(
            "SCRAM-Antwort des NAS: {expected} erwartet"
        )));
    }
    answer
        .get("rfc_str")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::Parse("SCRAM-Antwort des NAS ohne rfc_str".into()))
}

fn response_type(answer: &Value) -> Result<&str, ProviderError> {
    answer
        .get("response_type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::Parse("Antwort auf auth.login_ex ohne response_type".into()))
}

/// The error for a `response_type` other than the expected one.
fn unexpected(response_type: &str) -> ProviderError {
    match response_type {
        // Wrong or revoked key, expired key, user without API access.
        "AUTH_ERR" | "EXPIRED" | "DENIED" => ProviderError::LoginFailed,
        "OTP_REQUIRED" => ProviderError::SecondFactorRequired,
        // High availability: only the active controller logs in. Another
        // address may help, so no block.
        "REDIRECT" => ProviderError::Unreachable(
            "TrueNAS verweist auf den aktiven Controller, bitte dessen Adresse eintragen".into(),
        ),
        other => ProviderError::Parse(format!("Unbekannte Antwort auf auth.login_ex: {other}")),
    }
}
