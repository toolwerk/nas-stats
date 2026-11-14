//! Session handling against `authLogin.cgi`.

use std::fmt;

use nas_model::ProviderError;
use tokio::sync::Mutex;

use crate::auth::{LoginAnswer, encode_password, parse_login};
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
}

/// Client for one QNAP. It logs in on the first request, keeps the session
/// in memory and logs in again exactly once when the NAS reports the session
/// gone. After the NAS rejects a login it sends no further login until the
/// user acts ([`QnapClient::login`] or [`QnapClient::set_credentials`]).
pub struct QnapClient<T: Transport> {
    transport: T,
    state: Mutex<State>,
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
            }),
        }
    }

    /// `GET /cgi-bin/<path>` with the session, body of the `QDocRoot` answer.
    ///
    /// Logs in first when there is no session. When the NAS answers
    /// `authPassed` 0, logs in again once and repeats the request once; is
    /// it still 0, the account may not call this endpoint
    /// ([`ProviderError::PermissionDenied`]) and the session stays. While
    /// logins are blocked, returns the blocking error without a request.
    pub async fn get_xml(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<String, ProviderError> {
        let sid = {
            let mut state = self.state.lock().await;
            self.session(&mut state).await?
        };
        let body = self.send(path, query, &sid).await?;
        if !session_gone(&body)? {
            return Ok(body);
        }
        let sid = {
            let mut state = self.state.lock().await;
            if state.sid.as_deref() == Some(sid.as_str()) {
                state.sid = None;
            }
            // Another request may have logged in meanwhile; then its
            // session is used and no second login goes out.
            self.session(&mut state).await?
        };
        let body = self.send(path, query, &sid).await?;
        if session_gone(&body)? {
            return Err(ProviderError::PermissionDenied);
        }
        Ok(body)
    }

    /// User action „Erneut anmelden“: lifts the block and logs in once.
    pub async fn login(&self) -> Result<(), ProviderError> {
        let mut state = self.state.lock().await;
        state.blocked = None;
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
    }

    /// Ends the session at the NAS, for removing the NAS. Without a session
    /// nothing is sent. The session is forgotten even when the request
    /// fails; the answer itself is not checked and nothing is retried.
    pub async fn logout(&self) -> Result<(), ProviderError> {
        let Some(sid) = self.state.lock().await.sid.take() else {
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

    async fn send(
        &self,
        path: &str,
        query: &[(&str, &str)],
        sid: &str,
    ) -> Result<String, ProviderError> {
        let path = format!("/cgi-bin/{}", path.trim_start_matches('/'));
        let mut full = query.to_vec();
        full.push(("sid", sid));
        self.transport.get_text(&path, &full).await
    }
}

/// Whether a QTS answer says the session is not valid (`authPassed` 0).
fn session_gone(body: &str) -> Result<bool, ProviderError> {
    let doc = parse_qdoc(body)?;
    Ok(root_text(&doc, "authPassed").as_deref() == Some("0"))
}
