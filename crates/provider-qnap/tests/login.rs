//! Login, session renewal and the hard stop, against fixture answers.

use std::collections::VecDeque;
use std::sync::Mutex;

use nas_model::{BoxFuture, ProviderError};
use provider_qnap::{QnapClient, Transport};

const LOGIN_OK: &str = include_str!("fixtures/login.xml");
const LOGIN_WRONG: &str = include_str!("fixtures/login_wrong_password.xml");
const LOGIN_2SV: &str = include_str!("fixtures/login_2sv.xml");
const EXPIRED: &str = include_str!("fixtures/session_expired.xml");
const SYSINFO: &str = include_str!("fixtures/sysinfo.xml");
const LOGOUT: &str = include_str!("fixtures/logout.xml");

/// One request as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
struct Call {
    method: &'static str,
    path: String,
    params: Vec<(String, String)>,
}

/// Answers from a queue, records every request.
#[derive(Default)]
struct FakeTransport {
    answers: Mutex<VecDeque<Result<String, ProviderError>>>,
    calls: Mutex<Vec<Call>>,
}

impl FakeTransport {
    fn new(answers: Vec<Result<&str, ProviderError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into_iter().map(|a| a.map(str::to_owned)).collect()),
            calls: Mutex::default(),
        }
    }

    fn answer(
        &self,
        method: &'static str,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<String, ProviderError> {
        self.calls.lock().expect("calls").push(Call {
            method,
            path: path.to_owned(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        });
        self.answers
            .lock()
            .expect("answers")
            .pop_front()
            .expect("unexpected request")
    }
}

impl Transport for &FakeTransport {
    fn get_text<'a>(
        &'a self,
        path: &'a str,
        query: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let result = self.answer("GET", path, query);
        Box::pin(async move { result })
    }

    fn post_form_text<'a>(
        &'a self,
        path: &'a str,
        form: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let result = self.answer("POST", path, form);
        Box::pin(async move { result })
    }
}

fn calls(fake: &FakeTransport) -> Vec<Call> {
    fake.calls.lock().expect("calls").clone()
}

fn logins(fake: &FakeTransport) -> usize {
    calls(fake)
        .iter()
        .filter(|c| c.path == "/cgi-bin/authLogin.cgi")
        .count()
}

fn param<'a>(call: &'a Call, key: &str) -> Option<&'a str> {
    call.params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

const SYSINFO_PATH: &str = "management/manaRequest.cgi";
const SYSINFO_QUERY: &[(&str, &str)] = &[("subfunc", "sysinfo"), ("hd", "no"), ("multicpu", "1")];

#[tokio::test]
async fn logs_in_once_and_sends_the_session() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(SYSINFO), Ok(SYSINFO)]);
    let client = QnapClient::new(&fake, "monitor", "pässwort");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, SYSINFO_QUERY).await,
        Ok(SYSINFO.to_owned())
    );
    assert_eq!(
        client.get_xml(SYSINFO_PATH, SYSINFO_QUERY).await,
        Ok(SYSINFO.to_owned())
    );

    let calls = calls(&fake);
    assert_eq!(calls.len(), 3);
    let login = &calls[0];
    assert_eq!(
        (login.method, login.path.as_str()),
        ("POST", "/cgi-bin/authLogin.cgi")
    );
    assert_eq!(param(login, "user"), Some("monitor"));
    assert_eq!(param(login, "pwd"), Some("cMOkc3N3b3J0"));
    assert_eq!(param(login, "serviceKey"), Some("1"));
    for get in &calls[1..] {
        assert_eq!(
            (get.method, get.path.as_str()),
            ("GET", "/cgi-bin/management/manaRequest.cgi")
        );
        assert_eq!(param(get, "subfunc"), Some("sysinfo"));
        assert_eq!(param(get, "sid"), Some("REDACTED"));
    }
}

#[tokio::test]
async fn wrong_password_blocks_further_logins() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_WRONG)]);
    let client = QnapClient::new(&fake, "monitor", "falsch");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(calls(&fake).len(), 1);
}

#[tokio::test]
async fn second_factor_blocks_further_logins() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_2SV)]);
    let client = QnapClient::new(&fake, "admin", "geheim");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(calls(&fake).len(), 1);
}

#[tokio::test]
async fn http_401_on_login_blocks_like_a_rejection() {
    let fake = FakeTransport::new(vec![Err(ProviderError::LoginFailed)]);
    let client = QnapClient::new(&fake, "monitor", "x");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(calls(&fake).len(), 1);
}

#[tokio::test]
async fn expired_session_logs_in_again_once() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(EXPIRED), Ok(LOGIN_OK), Ok(SYSINFO)]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
    assert_eq!(logins(&fake), 2);
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn rejected_renewal_is_login_failed_and_blocks() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(EXPIRED), Ok(LOGIN_WRONG)]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(calls(&fake).len(), 3);
}

#[tokio::test]
async fn still_refused_after_renewal_is_permission_denied_without_block() {
    let user_answer = include_str!("fixtures/sysinfo.user.xml");
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(user_answer),
        Ok(LOGIN_OK),
        Ok(user_answer),
        Ok(SYSINFO),
    ]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(
        client
            .get_xml("sys/sysRequest.cgi", &[("subfunc", "firm_update")])
            .await,
        Err(ProviderError::PermissionDenied)
    );
    // The session stays: the next request goes out without a login.
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
    assert_eq!(logins(&fake), 2);
    assert_eq!(calls(&fake).len(), 5);
}

#[tokio::test]
async fn login_after_block_tries_once_more() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_WRONG), Ok(LOGIN_OK), Ok(SYSINFO)]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(client.login().await, Ok(()));
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
    assert_eq!(logins(&fake), 2);
}

#[tokio::test]
async fn set_credentials_lifts_the_block_without_a_request() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_WRONG), Ok(LOGIN_OK), Ok(SYSINFO)]);
    let client = QnapClient::new(&fake, "monitor", "falsch");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::LoginFailed)
    );
    client.set_credentials("monitor", "richtig").await;
    assert_eq!(calls(&fake).len(), 1);
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
    let calls = calls(&fake);
    assert_eq!(param(&calls[1], "pwd"), Some("cmljaHRpZw=="));
}

#[tokio::test]
async fn network_error_on_login_does_not_block() {
    let fake = FakeTransport::new(vec![
        Err(ProviderError::Timeout),
        Err(ProviderError::Unreachable("refused".into())),
        Ok(LOGIN_OK),
        Ok(SYSINFO),
    ]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::Timeout)
    );
    assert!(matches!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::Unreachable(_))
    ));
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
}

#[tokio::test]
async fn non_qts_answer_is_a_parse_error_without_block() {
    let fake = FakeTransport::new(vec![
        Ok("<html>Not Found</html>"),
        Ok(LOGIN_OK),
        Ok(SYSINFO),
    ]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert!(matches!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Err(ProviderError::Parse(_))
    ));
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
}

#[tokio::test]
async fn logout_ends_the_session() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(SYSINFO), Ok(LOGOUT)]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(
        client.get_xml(SYSINFO_PATH, &[]).await,
        Ok(SYSINFO.to_owned())
    );
    assert_eq!(client.logout().await, Ok(()));
    let calls = calls(&fake);
    let logout = &calls[2];
    assert_eq!(
        (logout.method, logout.path.as_str()),
        ("GET", "/cgi-bin/authLogout.cgi")
    );
    assert_eq!(param(logout, "logout"), Some("1"));
    assert_eq!(param(logout, "sid"), Some("REDACTED"));
    // Session forgotten: a second logout sends nothing.
    assert_eq!(client.logout().await, Ok(()));
    assert_eq!(fake.calls.lock().expect("calls").len(), 3);
}

#[tokio::test]
async fn logout_without_session_sends_nothing() {
    let fake = FakeTransport::new(vec![]);
    let client = QnapClient::new(&fake, "monitor", "pw");
    assert_eq!(client.logout().await, Ok(()));
    assert!(calls(&fake).is_empty());
}

#[test]
fn debug_shows_no_secrets() {
    let fake = FakeTransport::new(vec![]);
    let client = QnapClient::new(&fake, "monitor", "geheim");
    let text = format!("{client:?}");
    assert!(!text.contains("geheim"));
    assert!(!text.contains("monitor"));
}
