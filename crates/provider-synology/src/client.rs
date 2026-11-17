//! Session handling against `SYNO.API.Auth`.

use std::fmt;

use nas_model::ProviderError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::api::{
    AUTH_API, ApiInfo, Catalog, INFO_PATH, catalog_query, code_text, parse_catalog, parse_envelope,
};
use crate::auth::{DEVICE_NAME, LoginAnswer, parse_login};
use crate::transport::Transport;

/// Highest `SYNO.API.Auth` version the client speaks (DSM 7).
const AUTH_VERSION_MAX: u32 = 7;
/// Lowest version with `device_name` and the device token (DSM 6).
const AUTH_VERSION_MIN: u32 = 6;

/// DSM error: the account may not call this API (no admin).
const NO_PERMISSION: u32 = 105;
/// DSM errors that mean the session is gone: 106 timed out, 107 duplicate
/// login, 119 SID not found.
const SESSION_GONE: [u32; 3] = [106, 107, 119];

struct State {
    user: String,
    password: String,
    /// Token the NAS issued after a login with code; sent as `device_id`.
    device_token: Option<String>,
    sid: Option<String>,
    /// Only a catalog the NAS answered successfully.
    catalog: Option<Catalog>,
    /// The rejection that stopped logins until the user acts.
    blocked: Option<ProviderError>,
    /// Logged out for good (NAS removed): no further login.
    closed: bool,
}

/// What a data request brought back.
enum Fetched {
    Data(Value),
    /// The NAS reports the session gone.
    SessionGone,
}

/// Client for one Synology NAS. It reads the API catalog and logs in on the
/// first request, keeps the session in memory and logs in again exactly once
/// when the NAS reports the session gone. After the NAS rejects a login or
/// blocks the IP it sends nothing until the user acts
/// ([`SynologyClient::login`], [`SynologyClient::login_with_code`] or
/// [`SynologyClient::set_credentials`]).
pub struct SynologyClient<T: Transport> {
    transport: T,
    state: Mutex<State>,
}

impl<T: Transport> fmt::Debug for SynologyClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither the password nor the session id nor the device token
        // belongs in a log.
        f.debug_struct("SynologyClient").finish_non_exhaustive()
    }
}

impl<T: Transport> SynologyClient<T> {
    /// Client without a session; nothing is sent yet. `device_token` is the
    /// token a former [`SynologyClient::login_with_code`] returned.
    pub fn new(
        transport: T,
        user: impl Into<String>,
        password: impl Into<String>,
        device_token: Option<String>,
    ) -> Self {
        Self {
            transport,
            state: Mutex::new(State {
                user: user.into(),
                password: password.into(),
                device_token,
                sid: None,
                catalog: None,
                blocked: None,
                closed: false,
            }),
        }
    }

    /// Calls `method` of `api` with the session, `data` of the answer
    /// (`Value::Null` when the NAS sends none). `None` when the NAS does not
    /// offer `api`. `version` defaults to the highest the NAS speaks.
    ///
    /// Reads the catalog and logs in first when needed. When the NAS reports
    /// the session gone (106, 107, 119, or HTTP 401), logs in again once and
    /// repeats the request once; is the session still refused, the account
    /// may not call it ([`ProviderError::PermissionDenied`]). Error 105 is
    /// [`ProviderError::PermissionDenied`] at once, without a new login, and
    /// the session stays. While logins are blocked, returns the blocking
    /// error without a request; after [`SynologyClient::logout`],
    /// [`ProviderError::LoginFailed`].
    pub async fn call(
        &self,
        api: &str,
        method: &str,
        version: Option<u32>,
        params: &[(&str, &str)],
    ) -> Result<Option<Value>, ProviderError> {
        let (info, sid) = {
            let mut state = self.state.lock().await;
            Self::check_open(&state)?;
            let Some(info) = self.catalog(&mut state).await?.get(api).cloned() else {
                return Ok(None);
            };
            (info, self.session(&mut state).await?)
        };
        let version = version.unwrap_or(info.max_version).to_string();
        let request = Request {
            info: &info,
            api,
            method,
            version: &version,
            params,
        };
        if let Fetched::Data(data) = self.fetch(&request, &sid).await? {
            return Ok(Some(data));
        }
        let sid = {
            let mut state = self.state.lock().await;
            if state.sid.as_deref() == Some(sid.as_str()) {
                state.sid = None;
            }
            // Another request may have logged in meanwhile; then its session
            // is used and no second login goes out.
            self.session(&mut state).await?
        };
        match self.fetch(&request, &sid).await? {
            Fetched::Data(data) => Ok(Some(data)),
            // Logged out meanwhile: the refusal is the logout, not a right.
            Fetched::SessionGone if self.state.lock().await.closed => {
                Err(ProviderError::LoginFailed)
            }
            Fetched::SessionGone => Err(ProviderError::PermissionDenied),
        }
    }

    /// User action „Erneut anmelden“: lifts the block and logs in once,
    /// with the device token when there is one.
    pub async fn login(&self) -> Result<(), ProviderError> {
        let mut state = self.state.lock().await;
        Self::reopen(&mut state);
        self.catalog(&mut state).await?;
        self.login_locked(&mut state, None).await.map(|_| ())
    }

    /// User action: the code of the second factor, entered once. Lifts the
    /// block and logs in with `otp_code` and `enable_device_token=yes`.
    /// Returns the device token the NAS issued (`None` when it sent none);
    /// the caller stores it and hands it to [`SynologyClient::new`] or
    /// [`SynologyClient::set_credentials`] next time. The client itself
    /// stores nothing.
    pub async fn login_with_code(&self, code: &str) -> Result<Option<String>, ProviderError> {
        let mut state = self.state.lock().await;
        Self::reopen(&mut state);
        self.catalog(&mut state).await?;
        let (_, did) = self.login_locked(&mut state, Some(code)).await?;
        Ok(did)
    }

    /// User action „Zugang ändern“: new credentials and device token, no
    /// session, no block. Sends nothing.
    pub async fn set_credentials(
        &self,
        user: impl Into<String>,
        password: impl Into<String>,
        device_token: Option<String>,
    ) {
        let mut state = self.state.lock().await;
        state.user = user.into();
        state.password = password.into();
        state.device_token = device_token;
        Self::reopen(&mut state);
    }

    /// Ends the session at the NAS, for removing the NAS. Without a session
    /// nothing is sent, but the client still stays logged out. The session
    /// is forgotten even when the request fails; the answer is not checked
    /// and nothing is retried. Afterwards [`SynologyClient::call`] answers
    /// [`ProviderError::LoginFailed`] without a request, also for requests
    /// still running, until a `login*` or [`SynologyClient::set_credentials`].
    pub async fn logout(&self) -> Result<(), ProviderError> {
        let (sid, auth) = {
            let mut state = self.state.lock().await;
            state.closed = true;
            let auth = state
                .catalog
                .as_ref()
                .and_then(|c| c.get(AUTH_API))
                .cloned();
            (state.sid.take(), auth)
        };
        let (Some(sid), Some(auth)) = (sid, auth) else {
            return Ok(());
        };
        let path = api_path(&auth);
        let version = auth.max_version.min(AUTH_VERSION_MAX).to_string();
        let query = [
            ("api", AUTH_API),
            ("method", "logout"),
            ("version", version.as_str()),
            ("_sid", sid.as_str()),
        ];
        self.transport.get_text(&path, &query).await.map(|_| ())
    }

    /// No session, no block, not closed.
    fn reopen(state: &mut State) {
        state.sid = None;
        state.blocked = None;
        state.closed = false;
    }

    /// The blocking error, or `LoginFailed` after a logout.
    fn check_open(state: &State) -> Result<(), ProviderError> {
        if let Some(blocked) = &state.blocked {
            return Err(blocked.clone());
        }
        if state.closed {
            return Err(ProviderError::LoginFailed);
        }
        Ok(())
    }

    /// The API catalog, asked once; only a successful answer is kept.
    async fn catalog<'s>(&self, state: &'s mut State) -> Result<&'s Catalog, ProviderError> {
        if state.catalog.is_none() {
            let query = catalog_query();
            let params = [
                ("api", "SYNO.API.Info"),
                ("version", "1"),
                ("method", "query"),
                ("query", query.as_str()),
            ];
            let body = self.transport.get_text(INFO_PATH, &params).await?;
            state.catalog = Some(parse_catalog(&body)?);
        }
        state
            .catalog
            .as_ref()
            .ok_or_else(|| ProviderError::Parse("API-Katalog fehlt".into()))
    }

    /// Current session, logging in when there is none.
    async fn session(&self, state: &mut State) -> Result<String, ProviderError> {
        Self::check_open(state)?;
        match &state.sid {
            Some(sid) => Ok(sid.clone()),
            None => self.login_locked(state, None).await.map(|(sid, _)| sid),
        }
    }

    /// One login, with `code` as second factor when given, otherwise with
    /// the device token when there is one. A rejection by the NAS (also
    /// HTTP 401) blocks further logins; network and format errors do not.
    /// Session id and the device token of this answer; after a login with
    /// code the token replaces the stored one.
    async fn login_locked(
        &self,
        state: &mut State,
        code: Option<&str>,
    ) -> Result<(String, Option<String>), ProviderError> {
        let auth = state
            .catalog
            .as_ref()
            .and_then(|c| c.get(AUTH_API))
            .cloned()
            .ok_or_else(|| ProviderError::Parse("SYNO.API.Auth fehlt im API-Katalog".into()))?;
        let version = auth.max_version.min(AUTH_VERSION_MAX);
        if version < AUTH_VERSION_MIN {
            return Err(ProviderError::Parse(format!(
                "DSM-Version zu alt (SYNO.API.Auth Version {version}, nötig ab {AUTH_VERSION_MIN})"
            )));
        }
        let version = version.to_string();
        let mut form = vec![
            ("api", AUTH_API),
            ("method", "login"),
            ("version", version.as_str()),
            ("account", state.user.as_str()),
            ("passwd", state.password.as_str()),
            ("format", "sid"),
            ("device_name", DEVICE_NAME),
        ];
        match (code, &state.device_token) {
            (Some(code), _) => {
                form.push(("otp_code", code));
                form.push(("enable_device_token", "yes"));
            }
            (None, Some(token)) => form.push(("device_id", token.as_str())),
            (None, None) => {}
        }
        let answer = match self.transport.post_form_text(&api_path(&auth), &form).await {
            Ok(body) => parse_login(&body)?,
            Err(ProviderError::LoginFailed) => LoginAnswer::Refused(ProviderError::LoginFailed),
            Err(e) => return Err(e),
        };
        match answer {
            LoginAnswer::Session { sid, did } => {
                if code.is_some() && did.is_some() {
                    state.device_token.clone_from(&did);
                }
                state.sid = Some(sid.clone());
                Ok((sid, did))
            }
            LoginAnswer::Refused(error) => {
                state.blocked = Some(error.clone());
                Err(error)
            }
        }
    }

    /// One request with the session.
    async fn fetch(&self, request: &Request<'_>, sid: &str) -> Result<Fetched, ProviderError> {
        let mut query = vec![
            ("api", request.api),
            ("method", request.method),
            ("version", request.version),
        ];
        query.extend_from_slice(request.params);
        query.push(("_sid", sid));
        let body = match self
            .transport
            .get_text(&api_path(request.info), &query)
            .await
        {
            Ok(body) => body,
            Err(ProviderError::LoginFailed) => return Ok(Fetched::SessionGone),
            Err(e) => return Err(e),
        };
        let envelope = parse_envelope(&body)?;
        if envelope.success {
            return Ok(Fetched::Data(envelope.data.unwrap_or(Value::Null)));
        }
        match envelope.code {
            Some(NO_PERMISSION) => Err(ProviderError::PermissionDenied),
            Some(code) if SESSION_GONE.contains(&code) => Ok(Fetched::SessionGone),
            code => Err(ProviderError::Parse(format!(
                "{}.{}: DSM-Fehler {}",
                request.api,
                request.method,
                code_text(code)
            ))),
        }
    }
}

/// One data request.
struct Request<'a> {
    info: &'a ApiInfo,
    api: &'a str,
    method: &'a str,
    version: &'a str,
    params: &'a [(&'a str, &'a str)],
}

/// `/webapi/<path>` of an API.
fn api_path(info: &ApiInfo) -> String {
    format!("/webapi/{}", info.path.trim_start_matches('/'))
}
