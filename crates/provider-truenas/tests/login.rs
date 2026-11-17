//! Login against the fake TrueNAS: plain key for 25, SCRAM for 26, the
//! block after a rejected login, certificate and address checks.
#![allow(clippy::unwrap_used)]

mod common;

use common::{API_KEY, Config, Login, Version, credentials, start};
use nas_http::Fingerprint;
use provider_truenas::{ProviderError, TruenasClient, TruenasCredentials};
use serde_json::{Value, json};

fn client(server: &common::FakeTruenas) -> TruenasClient {
    TruenasClient::new(&server.url(), server.fingerprint(), credentials()).unwrap()
}

async fn system_info(client: &TruenasClient) -> Value {
    let mut session = client.session().await.unwrap();
    let info = session.call("system.info", json!([])).await.unwrap();
    session.close().await;
    info
}

#[tokio::test]
async fn truenas_25_logs_in_with_the_plain_key() {
    let server = start(Config::new(Version::V25)).await;
    let info = system_info(&client(&server)).await;
    assert_eq!(info["hostname"], "truenas-demo");
    assert_eq!(
        server.methods(),
        ["auth.mechanism_choices", "auth.login_ex", "system.info"]
    );
    let login: Value = serde_json::from_str(&server.log()[1]).unwrap();
    assert_eq!(
        login["params"],
        json!([{
            "mechanism": "API_KEY_PLAIN",
            "username": "nasstats",
            "api_key": API_KEY,
            "login_options": {"user_info": false},
        }])
    );
}

#[tokio::test]
async fn truenas_26_logs_in_with_bound_scram_and_never_sends_the_key() {
    let server = start(Config::new(Version::V26)).await;
    let client = client(&server);
    let info = system_info(&client).await;
    assert_eq!(info["version"], "TrueNAS-26.04.0");
    assert_eq!(
        server.methods(),
        [
            "auth.mechanism_choices",
            "auth.login_ex",
            "auth.login_ex",
            "system.info"
        ]
    );
    let secret = API_KEY.split_once('-').unwrap().1;
    for message in server.log() {
        assert!(!message.contains(secret), "{message}");
    }
    let first: Value = serde_json::from_str(&server.log()[1]).unwrap();
    assert_eq!(first["params"][0]["mechanism"], "SCRAM");
    assert_eq!(first["params"][0]["scram_type"], "CLIENT_FIRST_MESSAGE");
    assert!(
        first["params"][0]["rfc_str"]
            .as_str()
            .unwrap()
            .starts_with("p=tls-server-end-point,,n=nasstats:7,r="),
        "{first}"
    );
    // A second session uses the kept keys and logs in again.
    system_info(&client).await;
    assert_eq!(server.connections(), 2);
}

#[tokio::test]
async fn rejected_logins_block_without_further_connections() {
    for (version, login, expected) in [
        (Version::V25, Login::AuthErr, ProviderError::LoginFailed),
        (Version::V26, Login::AuthErr, ProviderError::LoginFailed),
        (
            Version::V26,
            Login::BadSignature,
            ProviderError::LoginFailed,
        ),
        (
            Version::V25,
            Login::OtpRequired,
            ProviderError::SecondFactorRequired,
        ),
        (Version::V25, Login::RpcError, ProviderError::LoginFailed),
        (Version::V26, Login::RpcError, ProviderError::LoginFailed),
    ] {
        let server = start(Config {
            login,
            ..Config::new(version)
        })
        .await;
        let client = client(&server);
        let err = client.session().await.unwrap_err();
        assert_eq!(err, expected, "{version:?} {login:?}");
        assert!(client.is_blocked());
        assert_eq!(client.session().await.unwrap_err(), expected);
        assert_eq!(server.connections(), 1, "{version:?} {login:?}");
    }
}

#[tokio::test]
async fn wrong_key_blocks_and_new_credentials_lift_the_block() {
    for version in [Version::V25, Version::V26] {
        let server = start(Config::new(version)).await;
        let wrong = TruenasCredentials::new("nasstats", "7-falsch").unwrap();
        let client = TruenasClient::new(&server.url(), server.fingerprint(), wrong).unwrap();
        assert_eq!(
            client.session().await.unwrap_err(),
            ProviderError::LoginFailed,
            "{version:?}"
        );
        client.set_credentials(credentials());
        assert!(!client.is_blocked());
        assert_eq!(system_info(&client).await["hostname"], "truenas-demo");
        assert_eq!(server.connections(), 2);
    }
}

#[tokio::test]
async fn redirect_is_unreachable_without_block() {
    let server = start(Config {
        login: Login::Redirect,
        ..Config::new(Version::V25)
    })
    .await;
    let client = client(&server);
    let err = client.session().await.unwrap_err();
    assert!(matches!(err, ProviderError::Unreachable(_)), "{err:?}");
    assert!(!client.is_blocked());
    client.session().await.unwrap_err();
    assert_eq!(server.connections(), 2);
}

#[tokio::test]
async fn foreign_certificate_is_certificate_changed() {
    let server = start(Config::new(Version::V26)).await;
    let pin = Fingerprint::of_der(b"anderes Zertifikat");
    let client = TruenasClient::new(&server.url(), pin, credentials()).unwrap();
    let err = client.session().await.unwrap_err();
    assert_eq!(
        err,
        ProviderError::CertificateChanged {
            old: pin.to_string(),
            new: server.fingerprint().to_string(),
        }
    );
    assert!(server.log().is_empty());
}

#[tokio::test]
async fn plain_addresses_are_refused_before_any_connection() {
    let pin = Fingerprint::of_der(b"cert");
    for url in [
        "http://127.0.0.1:1/",
        "ws://127.0.0.1:1/",
        "wss://127.0.0.1:1/",
    ] {
        let err = TruenasClient::new(url, pin, credentials()).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)), "{url}: {err:?}");
    }
}

#[tokio::test]
async fn refused_upgrade_maps_its_status() {
    for (status, expected) in [
        (404, ProviderError::Http(404)),
        (403, ProviderError::PermissionDenied),
    ] {
        let server = start(Config {
            upgrade_status: Some(status),
            ..Config::new(Version::V26)
        })
        .await;
        let client = client(&server);
        assert_eq!(client.session().await.unwrap_err(), expected, "{status}");
        assert!(!client.is_blocked());
    }
}

#[tokio::test]
async fn path_of_the_address_goes_before_the_endpoint() {
    // The fake server only answers on /api/current, so a path in front
    // reaches it as /nas/api/current and is refused with 404.
    let server = start(Config::new(Version::V26)).await;
    let url = format!("{}nas/", server.url());
    let client = TruenasClient::new(&url, server.fingerprint(), credentials()).unwrap();
    assert_eq!(
        client.session().await.unwrap_err(),
        ProviderError::Http(404)
    );
}
