//! Calls on a logged-in session against the fake TrueNAS.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::{Config, Version, credentials, start};
use provider_truenas::{ProviderError, Session, TruenasClient};
use serde_json::json;

async fn session(server: &common::FakeTruenas, timeout: Duration) -> Session {
    TruenasClient::with_timeout(&server.url(), server.fingerprint(), credentials(), timeout)
        .unwrap()
        .session()
        .await
        .unwrap()
}

#[tokio::test]
async fn events_and_foreign_ids_are_skipped() {
    let server = start(Config::new(Version::V25)).await;
    let mut session = session(&server, Duration::from_secs(5)).await;
    assert_eq!(
        session.call("test.noise", json!([])).await,
        Ok(json!("eigene"))
    );
    // The session goes on after the noise.
    let info = session.call("system.info", json!([])).await.unwrap();
    assert_eq!(info["hostname"], "truenas-demo");
}

#[tokio::test]
async fn error_objects_map_to_provider_errors() {
    let server = start(Config::new(Version::V25)).await;
    let mut session = session(&server, Duration::from_secs(5)).await;
    assert_eq!(
        session.call("test.eacces", json!([])).await,
        Err(ProviderError::PermissionDenied)
    );
    assert_eq!(
        session.call("pool.unbekannt", json!([])).await,
        Err(ProviderError::Http(404))
    );
}

#[tokio::test]
async fn silent_server_is_timeout() {
    let server = start(Config::new(Version::V25)).await;
    let mut session = session(&server, Duration::from_millis(300)).await;
    assert_eq!(
        session.call("test.silent", json!([])).await,
        Err(ProviderError::Timeout)
    );
}

#[tokio::test]
async fn answer_that_is_not_json_is_parse() {
    let server = start(Config::new(Version::V25)).await;
    let mut session = session(&server, Duration::from_secs(5)).await;
    let err = session.call("test.garbage", json!([])).await.unwrap_err();
    assert!(matches!(err, ProviderError::Parse(_)), "{err:?}");
}

#[tokio::test]
async fn closed_connection_is_unreachable() {
    let server = start(Config::new(Version::V25)).await;
    let mut session = session(&server, Duration::from_secs(5)).await;
    let err = session.call("test.close", json!([])).await.unwrap_err();
    assert!(matches!(err, ProviderError::Unreachable(_)), "{err:?}");
    let err = session.call("system.info", json!([])).await.unwrap_err();
    assert!(matches!(err, ProviderError::Unreachable(_)), "{err:?}");
}

#[tokio::test]
async fn calls_carry_ascending_ids_and_json_rpc_2() {
    let server = start(Config::new(Version::V25)).await;
    let mut session = session(&server, Duration::from_secs(5)).await;
    session.call("system.info", json!([])).await.unwrap();
    let calls: Vec<serde_json::Value> = server
        .log()
        .iter()
        .map(|m| serde_json::from_str(m).unwrap())
        .collect();
    let ids: Vec<_> = calls.iter().map(|c| c["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [1, 2, 3]);
    assert!(calls.iter().all(|c| c["jsonrpc"] == "2.0"));
}
