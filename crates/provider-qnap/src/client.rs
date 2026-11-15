//! Session handling against `authLogin.cgi`.

use std::fmt;

use nas_model::ProviderError;
use tokio::sync::Mutex;

use crate::auth::{LoginAnswer, encode_password, parse_login};
use crate::system::FirmwareCache;
use crate::transport::Transport;
use crate::xml::{parse_qdoc, root_text};

const LOGIN_PATH: &str = "/cgi-bin/authLogin.cgi";
const LOGOUT_PATH: &str = "/cgi-bin/authLogout.cgi";

struct State {
    user: String,
    password: String,
    sid: Option<String>,
    /// The rejection that stopped logins until the user acts.
    blocked: Option<ProviderError>,
    /// Logged out for good (NAS removed): no further login.
    closed: bool,
}

/// Client for one QNAP. It logs in on the first request, keeps the session
/// in memory and logs in again exactly once when the NAS reports the session
/// gone. After the NAS rejects a login it sends no further login until the
/// user acts ([`QnapClient::login`] or [`QnapClient::set_credentials`]).
pub struct QnapClient<T: Transport> {
    transport: T,
    state: Mutex<State>,
    /// Last answer of the firmware update request (see `system.rs`).
    pub(crate) firmware_cache: FirmwareCache,
}

impl<T: Transport> fmt::Debug for QnapClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither the password nor the session id belongs in a log.
        f.debug_struct("QnapClient").finish_non_exhaustive()
    }
}

impl<T: Transport> QnapClient<T> {
    /// Client without a session; nothing is sent yet.
    pub fn new(transport: T, user: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            transport,
            state: Mutex::new(State {
                user: user.into(),
                password: password.into(),
                sid: None,
                blocked: None,
                closed: false,
            }),
            firmware_cache: FirmwareCache::default(),
        }
    }

    /// `GET /cgi-bin/<path>` with the session, body of the `QDocRoot` answer.
    ///
    /// Logs in first when there is no session. When the NAS answers
    /// `authPassed` 0 or HTTP 401, logs in again once and repeats the
    /// request once; is it still refused, the account may not call this
    /// endpoint ([`ProviderError::PermissionDenied`]) and the session stays.
    /// While logins are blocked, returns the blocking error without a
    /// request; after [`QnapClient::logout`], [`ProviderError::LoginFailed`].
    pub async fn get_xml(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<String, ProviderError> {
        let sid = {
            let mut state = self.state.lock().await;
            self.session(&mut state).await?
        };
        if let Some(body) = self.fetch(path, query, &sid).await? {
            return Ok(body);
        }
        let sid = {
            let mut state = self.state.lock().await;
            if state.sid.as_deref() == Some(sid.as_str()) {
                state.sid = None;
            }
            // Another request may have logged in meanwhile; then its
            // session is used and no second login goes out. Only after a
            // network or format error on that login does the next request
            // try once more (such errors do not block).
            self.session(&mut state).await?
        };
        match self.fetch(path, query, &sid).await? {
            Some(body) => Ok(body),
            // Logged out meanwhile: the refusal is the logout, not a right.
            None if self.state.lock().await.closed => Err(ProviderError::LoginFailed),
            None => Err(ProviderError::PermissionDenied),
        }
    }

    /// User action „Erneut anmelden“: lifts the block and logs in once.
    pub async fn login(&self) -> Result<(), ProviderError> {
        let mut state = self.state.lock().await;
        state.blocked = None;
        state.closed = false;
        state.sid = None;
        self.login_locked(&mut state).await.map(|_| ())
    }

    /// User action „Zugang ändern“: new credentials, no session, no block.
    /// Sends nothing.
    pub async fn set_credentials(&self, user: impl Into<String>, password: impl Into<String>) {
        let mut state = self.state.lock().await;
        state.user = user.into();
        state.password = password.into();
        state.sid = None;
        state.blocked = None;
        state.closed = false;
    }

    /// Ends the session at the NAS, for removing the NAS. Without a session
    /// nothing is sent, but the client still stays logged out. The session is forgotten even when the request
    /// fails; the answer itself is not checked and nothing is retried.
    /// Afterwards [`QnapClient::get_xml`] logs in no more and answers
    /// [`ProviderError::LoginFailed`] without a request, also for requests
    /// still running, until [`QnapClient::login`] or
    /// [`QnapClient::set_credentials`].
    pub async fn logout(&self) -> Result<(), ProviderError> {
        let sid = {
            let mut state = self.state.lock().await;
            state.closed = true;
            state.sid.take()
        };
        let Some(sid) = sid else {
            return Ok(());
        };
        self.transport
            .get_text(LOGOUT_PATH, &[("logout", "1"), ("sid", &sid)])
            .await
            .map(|_| ())
    }

    /// Current session, logging in when there is none.
    async fn session(&self, state: &mut State) -> Result<String, ProviderError> {
        if let Some(blocked) = &state.blocked {
            return Err(blocked.clone());
        }
        if state.closed {
            return Err(ProviderError::LoginFailed);
        }
        match &state.sid {
            Some(sid) => Ok(sid.clone()),
            None => self.login_locked(state).await,
        }
    }

    /// One login. A rejection by the NAS blocks further logins; network and
    /// format errors do not.
    async fn login_locked(&self, state: &mut State) -> Result<String, ProviderError> {
        let pwd = encode_password(&state.password);
        let form = [
            ("user", state.user.as_str()),
            ("pwd", pwd.as_str()),
            ("serviceKey", "1"),
        ];
        let answer = match self.transport.post_form_text(LOGIN_PATH, &form).await {
            Ok(body) => parse_login(&body)?,
            Err(ProviderError::LoginFailed) => LoginAnswer::Rejected,
            Err(e) => return Err(e),
        };
        let error = match answer {
            LoginAnswer::Session(sid) => {
                state.sid = Some(sid.clone());
                return Ok(sid);
            }
            LoginAnswer::Rejected => ProviderError::LoginFailed,
            LoginAnswer::SecondFactor => ProviderError::SecondFactorRequired,
        };
        state.blocked = Some(error.clone());
        Err(error)
    }

    /// One request with the session; `None` when the NAS says the session
    /// is not valid (`authPassed` 0, or HTTP 401 which nas-http reports as
    /// [`ProviderError::LoginFailed`]).
    async fn fetch(
        &self,
        path: &str,
        query: &[(&str, &str)],
        sid: &str,
    ) -> Result<Option<String>, ProviderError> {
        let path = format!("/cgi-bin/{}", path.trim_start_matches('/'));
        let mut full = query.to_vec();
        full.push(("sid", sid));
        let body = match self.transport.get_text(&path, &full).await {
            Ok(body) => body,
            Err(ProviderError::LoginFailed) => return Ok(None),
            Err(e) => return Err(e),
        };
        let doc = parse_qdoc(&body)?;
        if root_text(&doc, "authPassed").as_deref() == Some("0") {
            return Ok(None);
        }
        Ok(Some(body))
    }
}
