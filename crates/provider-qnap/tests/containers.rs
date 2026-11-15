//! Containers from the recorded Container Station answers (issue #1) and
//! the cases where the list stays empty without an error.

mod common;

use common::{Call, FakeTransport, calls};
use nas_model::{Container, ProviderError};
use provider_qnap::QnapClient;

const CS_LOGIN: &str = include_str!("fixtures/cs_login.json");
const CS_LOGIN_USER: &str = include_str!("fixtures/cs_login.user.json");
const CS_CONTAINERS: &str = include_str!("fixtures/cs_container.json");

const CS_LOGIN_PATH: &str = "/container-station/api/v1/login";
const CS_LIST_PATH: &str = "/container-station/api/v1/container";

fn cs_logins(fake: &FakeTransport) -> usize {
    calls(fake)
        .iter()
        .filter(|c| c.path == CS_LOGIN_PATH)
        .count()
}

fn lists(fake: &FakeTransport) -> usize {
    calls(fake)
        .iter()
        .filter(|c| c.path == CS_LIST_PATH)
        .count()
}

#[tokio::test]
async fn reads_the_recorded_containers() {
    let fake = FakeTransport::new(vec![Ok(CS_LOGIN), Ok(CS_CONTAINERS)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let containers = client.containers().await.expect("containers");

    let expected: Vec<_> = (1..=6)
        .map(|i| Container {
            name: format!("container-{i}"),
            running: true,
        })
        .collect();
    assert_eq!(containers, expected);

    let calls = calls(&fake);
    assert_eq!(
        calls[0],
        Call {
            method: "POST",
            path: CS_LOGIN_PATH.into(),
            params: vec![(
                "json".into(),
                r#"{"password":"secret","username":"admin"}"#.into()
            )],
        }
    );
    assert_eq!(calls[1].method, "GET");
    assert_eq!(calls[1].path, CS_LIST_PATH);

    // The session is kept: the next poll only lists.
    fake.answers
        .lock()
        .expect("answers")
        .push_back(Ok(CS_CONTAINERS.into()));
    assert_eq!(client.containers().await.expect("containers").len(), 6);
    assert_eq!(cs_logins(&fake), 1);
}

#[tokio::test]
async fn an_account_without_admin_rights_is_asked_once() {
    let fake = FakeTransport::new(vec![Ok(CS_LOGIN_USER)]);
    let client = QnapClient::new(&fake, "user", "secret");
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(cs_logins(&fake), 1);
    assert_eq!(lists(&fake), 0);
}

#[tokio::test]
async fn a_rejected_login_is_not_repeated() {
    let fake = FakeTransport::new(vec![Err(ProviderError::LoginFailed)]);
    let client = QnapClient::new(&fake, "admin", "wrong");
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(cs_logins(&fake), 1);
}

#[tokio::test]
async fn a_missing_container_station_is_not_asked_again_soon() {
    let fake = FakeTransport::new(vec![Err(ProviderError::Http(404))]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(calls(&fake).len(), 1);
}

#[tokio::test]
async fn an_answer_that_is_no_json_means_not_installed() {
    let fake = FakeTransport::new(vec![Ok("<html>Not Found</html>")]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(calls(&fake).len(), 1);
}

#[tokio::test]
async fn a_gone_session_logs_in_exactly_once_more() {
    let fake = FakeTransport::new(vec![
        Ok(CS_LOGIN),
        Err(ProviderError::LoginFailed),
        Ok(CS_LOGIN),
        Err(ProviderError::LoginFailed),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(cs_logins(&fake), 2);
    assert_eq!(lists(&fake), 2);
    // No access now: the next poll sends nothing.
    assert_eq!(client.containers().await, Ok(Vec::new()));
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn a_gone_session_with_a_new_one_lists() {
    let fake = FakeTransport::new(vec![
        Ok(CS_LOGIN),
        Err(ProviderError::LoginFailed),
        Ok(CS_LOGIN),
        Ok(CS_CONTAINERS),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.containers().await.expect("containers").len(), 6);
    assert_eq!(cs_logins(&fake), 2);
}

#[tokio::test]
async fn new_credentials_log_in_again() {
    let fake = FakeTransport::new(vec![Ok(CS_LOGIN_USER), Ok(CS_LOGIN), Ok(CS_CONTAINERS)]);
    let client = QnapClient::new(&fake, "user", "secret");
    assert_eq!(client.containers().await, Ok(Vec::new()));
    client.set_credentials("admin", "secret").await;
    assert_eq!(client.containers().await.expect("containers").len(), 6);
    assert_eq!(cs_logins(&fake), 2);
}

#[tokio::test]
async fn a_timeout_is_returned_and_changes_nothing() {
    let fake = FakeTransport::new(vec![
        Err(ProviderError::Timeout),
        Ok(CS_LOGIN),
        Ok(CS_CONTAINERS),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.containers().await, Err(ProviderError::Timeout));
    assert_eq!(client.containers().await.expect("containers").len(), 6);
}

#[tokio::test]
async fn after_logout_nothing_is_sent() {
    let fake = FakeTransport::new(vec![]);
    let client = QnapClient::new(&fake, "admin", "secret");
    client.logout().await.expect("logout");
    assert_eq!(client.containers().await, Err(ProviderError::LoginFailed));
    assert!(calls(&fake).is_empty());
}
