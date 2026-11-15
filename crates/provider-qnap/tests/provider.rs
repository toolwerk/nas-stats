//! The QNAP snapshot from the recorded answers (issue #1).

mod common;

use common::{FakeTransport, logins};
use nas_model::{Alert, AlertLevel, Health, Provider, ProviderError, ProviderKind};
use provider_qnap::{QnapClient, QnapProvider};

const LOGIN_OK: &str = include_str!("fixtures/login.xml");
const LOGIN_WRONG: &str = include_str!("fixtures/login_wrong_password.xml");
const SYSINFO: &str = include_str!("fixtures/sysinfo.xml");
const HEALTH_WARNING: &str = include_str!("fixtures/syshealth.xml");
const FIRMWARE_NONE: &str = include_str!("fixtures/firmware.xml");

const BASE_URL: &str = "https://nas.example:443";

#[tokio::test]
async fn snapshot_holds_nas_system_fans_and_alerts() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_WARNING),
        Ok(FIRMWARE_NONE),
    ]);
    let provider = QnapProvider::new("nas-1", BASE_URL, QnapClient::new(&fake, "admin", "secret"));
    assert_eq!(provider.kind(), ProviderKind::Qnap);

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    let snapshot = provider.snapshot().await.expect("snapshot");

    let nas = &snapshot.nas;
    assert_eq!(nas.id, "nas-1");
    assert_eq!(nas.provider, ProviderKind::Qnap);
    assert_eq!(nas.base_url, BASE_URL);
    assert_eq!(nas.model.as_deref(), Some("TS-1685"));
    assert_eq!(nas.firmware.as_deref(), Some("5.2.11.3661 Build 20261023"));
    assert_eq!(nas.hostname.as_deref(), Some("nas-demo"));
    assert_eq!(nas.name.as_deref(), Some("nas-demo"));

    assert!(snapshot.observed_at >= before);
    assert_eq!(snapshot.system.cpu_percent, Some(24.8));
    assert_eq!(snapshot.system.temperature_c, Some(36.0));
    assert_eq!(snapshot.system.update_available, Some(false));
    assert_eq!(snapshot.fans.len(), 5);
    assert_eq!(snapshot.fans[0].name, "Lüfter 1");
    assert_eq!(snapshot.fans[0].rpm, Some(1019));
    assert_eq!(snapshot.fans[0].health, Some(Health::Ok));
    assert_eq!(
        snapshot.alerts,
        vec![Alert {
            level: AlertLevel::Warning,
            text: "warning".into(),
            since: None,
        }]
    );
    assert_eq!(snapshot.ups, None);
    assert!(snapshot.volumes.is_empty());
    assert!(snapshot.disks.is_empty());
    assert!(snapshot.backups.is_empty());
    assert!(snapshot.containers.is_empty());
    assert!(snapshot.interfaces.is_empty());
    assert_eq!(snapshot.worst_health(), Some(Health::Warning));
}

#[tokio::test]
async fn a_rejected_login_arrives_as_error() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_WRONG)]);
    let provider = QnapProvider::new("nas-1", BASE_URL, QnapClient::new(&fake, "admin", "wrong"));
    assert_eq!(
        provider.snapshot().await.expect_err("login refused"),
        ProviderError::LoginFailed
    );
    // The block of the client holds: no second login.
    assert_eq!(
        provider.snapshot().await.expect_err("still blocked"),
        ProviderError::LoginFailed
    );
    assert_eq!(logins(&fake), 1);
}
