//! Catalog, login, second factor with device token, session renewal and the
//! hard stop, against fixture answers.

mod common;

use common::{FakeTransport, calls, logins, param};
use nas_model::ProviderError;
use provider_synology::SynologyClient;
use serde_json::Value;

const API_INFO: &str = include_str!("fixtures/api_info.json");
const LOGIN_OK: &str = include_str!("fixtures/login.json");
const LOGIN_TOKEN: &str = include_str!("fixtures/login_device_token.json");
const LOGIN_2FA: &str = include_str!("fixtures/login_2fa.json");
const LOGIN_CODE_REJECTED: &str = include_str!("fixtures/login_code_rejected.json");
const LOGIN_WRONG: &str = include_str!("fixtures/login_wrong_password.json");
const LOGIN_BLOCKED: &str = include_str!("fixtures/login_ip_blocked.json");
const EXPIRED: &str = include_str!("fixtures/session_expired.json");
const NO_PERMISSION: &str = include_str!("fixtures/no_permission.json");
const DATA: &str = include_str!("fixtures/data.json");
const LOGOUT: &str = include_str!("fixtures/logout.json");

const SYSTEM: &str = "SYNO.Core.System";

fn data() -> Option<Value> {
    let body: Value = serde_json::from_str(DATA).expect("fixture");
    Some(body["data"].clone())
}

async fn info(client: &SynologyClient<&FakeTransport>) -> Result<Option<Value>, ProviderError> {
    client.call(SYSTEM, "info", None, &[]).await
}

#[tokio::test]
async fn reads_the_catalog_logs_in_once_and_sends_the_session() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(DATA), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pässwort", None);
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(
        client
            .call(SYSTEM, "info", Some(1), &[("type", "storage")])
            .await,
        Ok(data())
    );

    let calls = calls(&fake);
    assert_eq!(calls.len(), 4);
    let catalog = &calls[0];
    assert_eq!(
        (catalog.method, catalog.path.as_str()),
        ("GET", "/webapi/entry.cgi")
    );
    assert_eq!(param(catalog, "api"), Some("SYNO.API.Info"));
    assert_eq!(param(catalog, "version"), Some("1"));
    assert_eq!(param(catalog, "method"), Some("query"));
    assert_eq!(param(catalog, "query"), Some("SYNO.API.Auth"));

    let login = &calls[1];
    assert_eq!(
        (login.method, login.path.as_str()),
        ("POST", "/webapi/entry.cgi")
    );
    assert_eq!(param(login, "api"), Some("SYNO.API.Auth"));
    assert_eq!(param(login, "method"), Some("login"));
    assert_eq!(param(login, "version"), Some("7"));
    assert_eq!(param(login, "account"), Some("admin"));
    assert_eq!(param(login, "passwd"), Some("pässwort"));
    assert_eq!(param(login, "format"), Some("sid"));
    assert_eq!(param(login, "device_name"), Some("NAS Stats"));
    assert_eq!(param(login, "device_id"), None);
    assert_eq!(param(login, "otp_code"), None);

    let first = &calls[2];
    assert_eq!(
        (first.method, first.path.as_str()),
        ("GET", "/webapi/entry.cgi")
    );
    assert_eq!(param(first, "api"), Some(SYSTEM));
    assert_eq!(param(first, "method"), Some("info"));
    assert_eq!(param(first, "version"), Some("3"));
    assert_eq!(param(first, "_sid"), Some("REDACTED"));
    let second = &calls[3];
    assert_eq!(param(second, "version"), Some("1"));
    assert_eq!(param(second, "type"), Some("storage"));
    assert_eq!(param(second, "_sid"), Some("REDACTED"));
}

#[tokio::test]
async fn password_never_goes_into_a_query() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(DATA),
        Ok(EXPIRED),
        Ok(LOGIN_OK),
        Ok(DATA),
        Ok(LOGOUT),
    ]);
    let client = SynologyClient::new(&fake, "admin", "geheim", None);
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(client.logout().await, Ok(()));
    for call in calls(&fake) {
        let has_password = call.params.iter().any(|(_, v)| v == "geheim");
        assert_eq!(has_password, call.method == "POST", "{call:?}");
    }
}

#[tokio::test]
async fn api_missing_from_the_catalog_is_none_without_login() {
    let fake = FakeTransport::new(vec![Ok(API_INFO)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(
        client
            .call("SYNO.Docker.Container", "list", None, &[])
            .await,
        Ok(None)
    );
    assert_eq!(calls(&fake).len(), 1);
}

#[tokio::test]
async fn catalog_is_asked_once() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(DATA), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(info(&client).await, Ok(data()));
    let catalogs = calls(&fake)
        .iter()
        .filter(|c| param(c, "api") == Some("SYNO.API.Info"))
        .count();
    assert_eq!(catalogs, 1);
}

#[tokio::test]
async fn refused_catalog_is_a_parse_error_asked_again_without_block() {
    let fake = FakeTransport::new(vec![
        Ok(r#"{"error":{"code":102},"success":false}"#),
        Ok("<html>Not Found</html>"),
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert!(matches!(info(&client).await, Err(ProviderError::Parse(_))));
    assert!(matches!(info(&client).await, Err(ProviderError::Parse(_))));
    assert_eq!(info(&client).await, Ok(data()));
}

#[tokio::test]
async fn dsm_older_than_6_is_a_parse_error_without_login() {
    let old = r#"{"data":{"SYNO.API.Auth":{"maxVersion":3,"minVersion":1,"path":"auth.cgi"},"SYNO.Core.System":{"maxVersion":1,"minVersion":1,"path":"entry.cgi"}},"success":true}"#;
    let fake = FakeTransport::new(vec![Ok(old)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    match info(&client).await {
        Err(ProviderError::Parse(text)) => assert!(text.contains("zu alt"), "{text}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(logins(&fake), 0);
}

#[tokio::test]
async fn auth_path_and_version_come_from_the_catalog() {
    let dsm6 = r#"{"data":{"SYNO.API.Auth":{"maxVersion":6,"minVersion":1,"path":"auth.cgi"},"SYNO.Core.System":{"maxVersion":1,"minVersion":1,"path":"entry.cgi"}},"success":true}"#;
    let fake = FakeTransport::new(vec![Ok(dsm6), Ok(LOGIN_OK), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Ok(data()));
    let login = &calls(&fake)[1];
    assert_eq!(login.path, "/webapi/auth.cgi");
    assert_eq!(param(login, "version"), Some("6"));
}

#[tokio::test]
async fn second_factor_blocks_until_the_user_acts() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_2FA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(
        info(&client).await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(
        info(&client).await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(calls(&fake).len(), 2);
}

#[tokio::test]
async fn wrong_password_blocks_further_requests() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_WRONG)]);
    let client = SynologyClient::new(&fake, "admin", "falsch", None);
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(
        client
            .call("SYNO.Docker.Container", "list", None, &[])
            .await,
        Err(ProviderError::LoginFailed)
    );
    assert_eq!(calls(&fake).len(), 2);
}

#[tokio::test]
async fn ip_block_stops_every_request() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_BLOCKED)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::IpBlocked));
    assert_eq!(info(&client).await, Err(ProviderError::IpBlocked));
    assert_eq!(
        client
            .call("SYNO.Docker.Container", "list", None, &[])
            .await,
        Err(ProviderError::IpBlocked)
    );
    assert_eq!(calls(&fake).len(), 2);
    assert!(ProviderError::IpBlocked.stops_polling());
}

#[tokio::test]
async fn ip_block_on_renewal_blocks() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(LOGIN_BLOCKED),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::IpBlocked));
    assert_eq!(info(&client).await, Err(ProviderError::IpBlocked));
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn http_401_on_login_blocks_like_a_rejection() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Err(ProviderError::LoginFailed)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(calls(&fake).len(), 2);
}

#[tokio::test]
async fn network_and_format_errors_on_login_do_not_block() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Err(ProviderError::Timeout),
        Ok("<html>Bad Gateway</html>"),
        Ok(LOGIN_OK),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::Timeout));
    assert!(matches!(info(&client).await, Err(ProviderError::Parse(_))));
    assert_eq!(info(&client).await, Ok(data()));
}

#[tokio::test]
async fn login_lifts_the_block_and_tries_once() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_WRONG), Ok(LOGIN_OK), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(client.login().await, Ok(()));
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(logins(&fake), 2);
}

#[tokio::test]
async fn login_after_ip_block_tries_once_and_blocks_again() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_BLOCKED), Ok(LOGIN_BLOCKED)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::IpBlocked));
    assert_eq!(client.login().await, Err(ProviderError::IpBlocked));
    assert_eq!(info(&client).await, Err(ProviderError::IpBlocked));
    assert_eq!(logins(&fake), 2);
}

#[tokio::test]
async fn code_login_returns_the_device_token_used_by_the_next_login() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_2FA),
        Ok(LOGIN_TOKEN),
        Ok(DATA),
        Ok(EXPIRED),
        Ok(LOGIN_OK),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(
        info(&client).await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(
        client.login_with_code("123456").await,
        Ok(Some("REDACTED-DID".into()))
    );
    assert_eq!(info(&client).await, Ok(data()));
    // Session gone: the renewal uses the token, no code.
    assert_eq!(info(&client).await, Ok(data()));

    let calls = calls(&fake);
    let with_code = &calls[2];
    assert_eq!(param(with_code, "otp_code"), Some("123456"));
    assert_eq!(param(with_code, "enable_device_token"), Some("yes"));
    assert_eq!(param(with_code, "device_name"), Some("NAS Stats"));
    assert_eq!(param(with_code, "device_id"), None);
    let renewal = &calls[5];
    assert_eq!(param(renewal, "method"), Some("login"));
    assert_eq!(param(renewal, "device_id"), Some("REDACTED-DID"));
    assert_eq!(param(renewal, "otp_code"), None);
    assert_eq!(param(renewal, "enable_device_token"), None);
}

#[tokio::test]
async fn stored_device_token_logs_in_without_code() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", Some("REDACTED-DID".into()));
    assert_eq!(info(&client).await, Ok(data()));
    let login = &calls(&fake)[1];
    assert_eq!(param(login, "device_id"), Some("REDACTED-DID"));
    assert_eq!(param(login, "device_name"), Some("NAS Stats"));
    assert_eq!(param(login, "otp_code"), None);
}

#[tokio::test]
async fn rejected_code_blocks_until_the_next_code() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_CODE_REJECTED),
        Ok(LOGIN_TOKEN),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(
        client.login_with_code("000000").await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(
        info(&client).await,
        Err(ProviderError::SecondFactorRequired)
    );
    assert_eq!(calls(&fake).len(), 2);
    assert_eq!(
        client.login_with_code("123456").await,
        Ok(Some("REDACTED-DID".into()))
    );
    assert_eq!(info(&client).await, Ok(data()));
}

#[tokio::test]
async fn code_login_without_token_returns_none() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK)]);
    let client = SynologyClient::new(&fake, "admin", "pw", Some("alt".into()));
    assert_eq!(client.login_with_code("123456").await, Ok(None));
}

#[tokio::test]
async fn set_credentials_lifts_the_block_without_a_request() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_WRONG), Ok(LOGIN_OK), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "falsch", None);
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    client
        .set_credentials("admin", "richtig", Some("REDACTED-DID".into()))
        .await;
    assert_eq!(calls(&fake).len(), 2);
    assert_eq!(info(&client).await, Ok(data()));
    let login = &calls(&fake)[2];
    assert_eq!(param(login, "passwd"), Some("richtig"));
    assert_eq!(param(login, "device_id"), Some("REDACTED-DID"));
}

#[tokio::test]
async fn expired_session_logs_in_again_once() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(LOGIN_OK),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(logins(&fake), 2);
    assert_eq!(calls(&fake).len(), 5);
}

#[tokio::test]
async fn timed_out_and_duplicate_sessions_renew_too() {
    for code in [106, 107] {
        let gone = format!(r#"{{"error":{{"code":{code}}},"success":false}}"#);
        let fake = FakeTransport::new(vec![
            Ok(API_INFO),
            Ok(LOGIN_OK),
            Ok(&gone),
            Ok(LOGIN_OK),
            Ok(DATA),
        ]);
        let client = SynologyClient::new(&fake, "admin", "pw", None);
        assert_eq!(info(&client).await, Ok(data()), "{code}");
        assert_eq!(logins(&fake), 2, "{code}");
    }
}

#[tokio::test]
async fn http_401_on_a_request_renews_the_session_once() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Err(ProviderError::LoginFailed),
        Ok(LOGIN_OK),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(logins(&fake), 2);
}

#[tokio::test]
async fn still_expired_after_renewal_is_permission_denied_without_block() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::PermissionDenied));
    // The session stays: the next request goes out without a login.
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(logins(&fake), 2);
    assert_eq!(calls(&fake).len(), 6);
}

#[tokio::test]
async fn rejected_renewal_blocks() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(LOGIN_WRONG),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn no_permission_is_denied_at_once_and_the_session_stays() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(NO_PERMISSION),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "monitor", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::PermissionDenied));
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(logins(&fake), 1);
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn other_dsm_errors_are_parse_errors_with_api_and_code() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(r#"{"error":{"code":103},"success":false}"#),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(
        info(&client).await,
        Err(ProviderError::Parse(
            "SYNO.Core.System.info: DSM-Fehler 103".into()
        ))
    );
}

#[tokio::test]
async fn success_without_data_is_null() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(r#"{"success":true}"#)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Ok(Some(Value::Null)));
}

#[tokio::test]
async fn logout_ends_the_session() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(DATA), Ok(LOGOUT)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Ok(data()));
    assert_eq!(client.logout().await, Ok(()));
    let all = calls(&fake);
    let logout = &all[3];
    assert_eq!(
        (logout.method, logout.path.as_str()),
        ("GET", "/webapi/entry.cgi")
    );
    assert_eq!(param(logout, "api"), Some("SYNO.API.Auth"));
    assert_eq!(param(logout, "method"), Some("logout"));
    assert_eq!(param(logout, "_sid"), Some("REDACTED"));
    // Session forgotten: a second logout sends nothing, and no request
    // goes out until the user acts.
    assert_eq!(client.logout().await, Ok(()));
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn logout_without_session_sends_nothing() {
    let fake = FakeTransport::new(vec![]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(client.logout().await, Ok(()));
    assert_eq!(info(&client).await, Err(ProviderError::LoginFailed));
    assert!(calls(&fake).is_empty());
}

#[tokio::test]
async fn login_after_logout_opens_again() {
    let fake = FakeTransport::new(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(client.logout().await, Ok(()));
    assert_eq!(client.login().await, Ok(()));
    assert_eq!(info(&client).await, Ok(data()));
}

#[tokio::test]
async fn logout_during_a_request_opens_no_new_session() {
    let fake = FakeTransport::slow(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(EXPIRED), Ok(LOGOUT)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    // Catalog and session first, so the logout has a session to end.
    let (request, logout) = tokio::join!(info(&client), async {
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        client.logout().await
    });
    assert_eq!(logout, Ok(()));
    assert_eq!(request, Err(ProviderError::LoginFailed));
    assert_eq!(logins(&fake), 1);
}

#[tokio::test]
async fn concurrent_requests_log_in_once() {
    let fake = FakeTransport::slow(vec![Ok(API_INFO), Ok(LOGIN_OK), Ok(DATA), Ok(DATA)]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    let (a, b) = tokio::join!(info(&client), info(&client));
    assert_eq!(a, Ok(data()));
    assert_eq!(b, Ok(data()));
    assert_eq!(logins(&fake), 1);
    assert_eq!(calls(&fake).len(), 4);
}

#[tokio::test]
async fn concurrent_requests_renew_the_session_once() {
    let second = LOGIN_OK.replace("\"REDACTED\"", "\"second\"");
    let fake = FakeTransport::slow(vec![
        Ok(API_INFO),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(EXPIRED),
        Ok(&second),
        Ok(DATA),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    let (a, b) = tokio::join!(info(&client), info(&client));
    assert_eq!(a, Ok(data()));
    assert_eq!(b, Ok(data()));
    assert_eq!(logins(&fake), 2);
    assert_eq!(param(&calls(&fake)[6], "_sid"), Some("second"));
}

#[test]
fn debug_shows_no_secrets() {
    let fake = FakeTransport::new(vec![]);
    let client = SynologyClient::new(&fake, "admin", "geheim", Some("token".into()));
    let text = format!("{client:?}");
    for secret in ["geheim", "admin", "token"] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[test]
fn hints_are_german_and_name_the_way_out() {
    assert!(provider_synology::ADMIN_HINT.contains("administrators"));
    assert!(provider_synology::SECOND_FACTOR_HINT.contains("NAS Stats"));
    assert!(provider_synology::IP_BLOCKED_HINT.contains("Sicherheit › Schutz"));
    assert_eq!(provider_synology::DEVICE_NAME, "NAS Stats");
}

#[tokio::test]
async fn http_403_and_429_on_login_block_like_an_ip_block() {
    for status in [ProviderError::PermissionDenied, ProviderError::Http(429)] {
        let fake = FakeTransport::new(vec![Ok(API_INFO), Err(status.clone())]);
        let client = SynologyClient::new(&fake, "admin", "pw", None);
        assert_eq!(
            info(&client).await,
            Err(ProviderError::IpBlocked),
            "{status}"
        );
        assert_eq!(
            info(&client).await,
            Err(ProviderError::IpBlocked),
            "{status}"
        );
        assert_eq!(calls(&fake).len(), 2, "{status}");
    }
}

#[tokio::test]
async fn server_errors_on_login_do_not_block() {
    let fake = FakeTransport::new(vec![
        Ok(API_INFO),
        Err(ProviderError::Http(503)),
        Ok(LOGIN_OK),
        Ok(DATA),
    ]);
    let client = SynologyClient::new(&fake, "admin", "pw", None);
    assert_eq!(info(&client).await, Err(ProviderError::Http(503)));
    assert_eq!(info(&client).await, Ok(data()));
}
