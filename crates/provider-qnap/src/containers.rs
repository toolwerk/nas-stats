//! Containers of the Container Station.
//!
//! Sources, read-only:
//!
//! - `POST /container-station/api/v1/login` with the QTS user name and
//!   password as JSON. The Container Station has its own login and keeps
//!   the session in a cookie (the HTTP layer stores it per NAS).
//! - `GET /container-station/api/v1/container`
//!
//! Only admin accounts may read the containers (issue #1). A missing
//! Container Station, an account without admin rights and a rejected login
//! all give an empty list, never an error. They are remembered, so the
//! NAS does not see a login on every poll: no access until the user logs in
//! again or changes the credentials, a missing Container Station for
//! [`CONTAINER_RECHECK`].

use std::time::{Duration, SystemTime};

use nas_model::{Container, ProviderError};
use serde_json::Value;

use crate::client::QnapClient;
use crate::transport::Transport;

/// How long a missing Container Station is believed before it is looked
/// for again (it may be installed meanwhile).
pub const CONTAINER_RECHECK: Duration = Duration::from_secs(3600);

const LOGIN_PATH: &str = "/container-station/api/v1/login";
const LIST_PATH: &str = "/container-station/api/v1/container";

/// What the client knows about the Container Station of its NAS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CsState {
    /// Not asked yet, or the account changed.
    Unknown,
    /// Logged in; the session cookie is in the HTTP layer.
    Session,
    /// The account may not read containers, or its login was rejected.
    NoAccess,
    /// No Container Station found, at this time (wall clock).
    NotInstalled(SystemTime),
}

/// Outcome of one request to the Container Station.
enum Step<B> {
    Done(B),
    /// No list: the state says why.
    Empty,
}

impl<T: Transport> QnapClient<T> {
    /// Reads the containers with name and whether they run, in the order
    /// the Container Station lists them.
    ///
    /// Logs in to the Container Station when there is no session, and once
    /// more when the list request reports the session gone. While QTS
    /// logins are blocked or after [`QnapClient::logout`] it sends nothing
    /// and returns that error. Timeout, network and other HTTP errors are
    /// returned and change nothing that is remembered.
    pub async fn containers(&self) -> Result<Vec<Container>, ProviderError> {
        // Held for the whole call: two polls must not log in twice.
        let mut cs = self.container_station.lock().await;
        let (user, password) = self.credentials().await?;
        match *cs {
            CsState::NoAccess => return Ok(Vec::new()),
            CsState::NotInstalled(at)
                if at
                    .elapsed()
                    .is_ok_and(|elapsed| elapsed < CONTAINER_RECHECK) =>
            {
                return Ok(Vec::new());
            }
            CsState::Session => {}
            CsState::Unknown | CsState::NotInstalled(_) => {
                if let Step::Empty = self.cs_login(&mut cs, &user, &password).await? {
                    return Ok(Vec::new());
                }
            }
        }
        if let Step::Done(list) = self.cs_list(&mut cs, false).await? {
            return Ok(list);
        }
        if *cs != CsState::Unknown {
            return Ok(Vec::new());
        }
        // The session is gone: exactly one new login.
        if let Step::Empty = self.cs_login(&mut cs, &user, &password).await? {
            return Ok(Vec::new());
        }
        match self.cs_list(&mut cs, true).await? {
            Step::Done(list) => Ok(list),
            Step::Empty => Ok(Vec::new()),
        }
    }

    /// One login. Sets the state from the answer; only errors that say
    /// nothing about the Container Station are returned.
    async fn cs_login(
        &self,
        cs: &mut CsState,
        user: &str,
        password: &str,
    ) -> Result<Step<()>, ProviderError> {
        let body = serde_json::json!({ "username": user, "password": password }).to_string();
        match self.transport.post_json_text(LOGIN_PATH, &body).await {
            Ok(answer) => match serde_json::from_str::<Value>(&answer) {
                // Not an admin, or an error object like the one the list
                // answers for such accounts (`code`, `message`): no access.
                Ok(answer)
                    if answer.get("isAdmin").and_then(Value::as_bool) == Some(false)
                        || answer.get("code").is_some() =>
                {
                    *cs = CsState::NoAccess;
                    Ok(Step::Empty)
                }
                Ok(_) => {
                    *cs = CsState::Session;
                    Ok(Step::Done(()))
                }
                Err(_) => not_installed(cs),
            },
            // A rejected login is not repeated: it may count towards the
            // lock of the account at the NAS.
            Err(ProviderError::LoginFailed | ProviderError::PermissionDenied) => {
                *cs = CsState::NoAccess;
                Ok(Step::Empty)
            }
            Err(ProviderError::Http(status)) if is_missing(status) => not_installed(cs),
            Err(e) => Err(e),
        }
    }

    /// One list request. A gone session sets [`CsState::Unknown`] on the
    /// first try and [`CsState::NoAccess`] on the `retry`.
    async fn cs_list(
        &self,
        cs: &mut CsState,
        retry: bool,
    ) -> Result<Step<Vec<Container>>, ProviderError> {
        match self.transport.get_text(LIST_PATH, &[]).await {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(list) => Ok(Step::Done(parse_containers(&list))),
                Err(_) => not_installed(cs),
            },
            Err(ProviderError::LoginFailed) if !retry => {
                *cs = CsState::Unknown;
                Ok(Step::Empty)
            }
            Err(ProviderError::LoginFailed | ProviderError::PermissionDenied) => {
                *cs = CsState::NoAccess;
                Ok(Step::Empty)
            }
            Err(ProviderError::Http(status)) if is_missing(status) => not_installed(cs),
            Err(e) => Err(e),
        }
    }
}

/// HTTP status that means there is no Container Station at this path: not
/// found, or a redirect (QTS sends unknown paths to its start page).
fn is_missing(status: u16) -> bool {
    status == 404 || (300..400).contains(&status)
}

fn not_installed<B>(cs: &mut CsState) -> Result<Step<B>, ProviderError> {
    *cs = CsState::NotInstalled(SystemTime::now());
    Ok(Step::Empty)
}

/// The containers of a list answer: each object with a non-empty `name`
/// and a `state`, running when the state is `running`. Anything else is
/// left out; an answer that is no array gives none.
pub(crate) fn parse_containers(list: &Value) -> Vec<Container> {
    let Some(list) = list.as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.trim();
            let state = entry.get("state")?.as_str()?;
            (!name.is_empty()).then(|| Container {
                name: name.to_owned(),
                running: state == "running",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_without_name_or_state_are_left_out() {
        let list = serde_json::json!([
            { "name": "a", "state": "running" },
            { "name": "b", "state": "exited" },
            { "name": "", "state": "running" },
            { "name": "c" },
            { "state": "running" },
            "kein Objekt",
        ]);
        assert_eq!(
            parse_containers(&list),
            vec![
                Container {
                    name: "a".into(),
                    running: true,
                },
                Container {
                    name: "b".into(),
                    running: false,
                },
            ]
        );
    }

    #[test]
    fn no_array_gives_no_containers() {
        assert!(parse_containers(&serde_json::json!({ "code": 1002 })).is_empty());
    }
}
