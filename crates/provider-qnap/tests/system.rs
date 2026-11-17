//! System values from the recorded QNAP answers (issue #1) and a few
//! rebuilt edge cases.

mod common;

use common::{FakeTransport, calls, param};
use nas_model::{Alert, AlertLevel, Fan, Health, ProviderError, Ups, UpsState};
use provider_qnap::QnapClient;

const LOGIN_OK: &str = include_str!("fixtures/login.xml");
const EXPIRED: &str = include_str!("fixtures/session_expired.xml");
const SYSINFO: &str = include_str!("fixtures/sysinfo.xml");
const SYSINFO_EMPTY: &str = include_str!("fixtures/sysinfo_empty.xml");
const HEALTH_WARNING: &str = include_str!("fixtures/syshealth.xml");
const HEALTH_GOOD: &str = include_str!("fixtures/syshealth_good.xml");
const UPS: &str = include_str!("fixtures/ups.xml");
const UPS_ON_BATTERY: &str = include_str!("fixtures/ups_on_battery.xml");
const UPS_NONE: &str = include_str!("fixtures/ups_none.xml");
const FIRMWARE_NONE: &str = include_str!("fixtures/firmware.xml");
const FIRMWARE_AVAILABLE: &str = include_str!("fixtures/firmware_available.xml");

fn fan(name: &str, rpm: u32) -> Fan {
    Fan {
        name: name.into(),
        rpm: Some(rpm),
        health: Some(Health::Ok),
    }
}

#[tokio::test]
async fn reads_the_recorded_answers() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_WARNING),
        Ok(UPS),
        Ok(FIRMWARE_NONE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");

    assert_eq!(system.model.as_deref(), Some("TS-1685"));
    assert_eq!(
        system.firmware.as_deref(),
        Some("5.2.11.3661 Build 20261023")
    );
    assert_eq!(system.hostname.as_deref(), Some("nas-demo"));

    let status = &system.status;
    assert_eq!(
        status.uptime_seconds,
        Some(7 * 86_400 + 5 * 3_600 + 44 * 60)
    );
    assert_eq!(status.cpu_percent, Some(24.8));
    assert_eq!(status.memory_total_bytes, Some(15_909 * 1_048_576));
    let memory = status.memory_percent.expect("memory percent");
    assert!((memory - (15_909.0 - 13_435.7) / 15_909.0 * 100.0).abs() < 1e-9);
    assert_eq!(status.temperature_c, Some(36.0));
    assert_eq!(status.cpu_temperature_c, Some(65.0));
    assert_eq!(status.update_available, Some(false));
    assert_eq!(status.update_version, None);

    assert_eq!(
        system.fans,
        vec![
            fan("Lüfter 1", 1019),
            fan("Lüfter 2", 994),
            fan("Lüfter 3", 1002),
            fan("CPU-Lüfter 1", 3477),
            fan("CPU-Lüfter 2", 3199),
        ]
    );
    assert_eq!(
        system.ups,
        Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: Some(100.0),
            runtime_seconds: Some(3_720),
        })
    );
    assert_eq!(
        system.alerts,
        vec![Alert {
            level: AlertLevel::Warning,
            text: "warning".into(),
            since: None,
        }]
    );

    let calls = calls(&fake);
    let requests: Vec<_> = calls
        .iter()
        .skip(1)
        .map(|c| {
            (
                c.path.as_str(),
                param(c, "subfunc").unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        requests,
        vec![
            ("/cgi-bin/management/manaRequest.cgi", "sysinfo".to_owned()),
            ("/cgi-bin/management/manaRequest.cgi", "sysinfo".to_owned()),
            ("/cgi-bin/devices/devRequest.cgi", "ups".to_owned()),
            ("/cgi-bin/sys/sysRequest.cgi", "firm_update".to_owned()),
        ]
    );
    assert_eq!(param(&calls[1], "hd"), Some("no"));
    assert_eq!(param(&calls[1], "multicpu"), Some("1"));
    assert_eq!(param(&calls[2], "sysHealth"), Some("1"));
}

#[tokio::test]
async fn empty_fields_stay_empty() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO_EMPTY),
        Ok(HEALTH_GOOD),
        Ok(UPS_NONE),
        Ok(FIRMWARE_NONE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");

    assert_eq!(system.model, None);
    assert_eq!(system.firmware, None);
    assert_eq!(system.hostname, None);
    let status = &system.status;
    assert_eq!(status.uptime_seconds, None);
    assert_eq!(status.cpu_percent, None);
    assert_eq!(status.memory_percent, None);
    assert_eq!(status.memory_total_bytes, None);
    assert_eq!(status.temperature_c, None);
    assert_eq!(status.cpu_temperature_c, None);
    assert!(system.fans.is_empty());
    assert_eq!(system.ups, None);
    assert!(system.alerts.is_empty());
}

#[tokio::test]
async fn an_offered_update_names_its_version() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_AVAILABLE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let status = client.system().await.expect("system").status;
    assert_eq!(status.update_available, Some(true));
    assert_eq!(
        status.update_version.as_deref(),
        Some("5.2.12.3700 Build 20261201")
    );
}

#[tokio::test]
async fn a_refused_part_only_leaves_its_value_empty() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Err(ProviderError::PermissionDenied),
        Err(ProviderError::PermissionDenied),
        Err(ProviderError::PermissionDenied),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");
    assert_eq!(system.model.as_deref(), Some("TS-1685"));
    assert!(system.alerts.is_empty());
    assert_eq!(system.status.update_available, None);
    assert_eq!(system.status.update_version, None);
}

#[tokio::test]
async fn a_part_refused_with_auth_passed_0_logs_in_once_and_goes_on() {
    // QTS refuses with HTTP 200 and `authPassed` 0 (issue #1): one new
    // login, one repeat, then the part stays empty and the next part is
    // still asked.
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(EXPIRED),
        Ok(LOGIN_OK),
        Ok(EXPIRED),
        Ok(UPS),
        Ok(FIRMWARE_AVAILABLE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");
    assert!(system.alerts.is_empty());
    assert_eq!(system.status.update_available, Some(true));
    assert_eq!(calls(&fake).len(), 7);
}

#[tokio::test]
async fn an_unreadable_part_only_leaves_its_value_empty() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok("<html><body>Not found</body></html>"),
        Ok(UPS),
        Ok(FIRMWARE_NONE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");
    assert!(system.alerts.is_empty());
    assert_eq!(system.status.update_available, Some(false));
}

#[tokio::test]
async fn new_credentials_fetch_the_firmware_again() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Err(ProviderError::PermissionDenied),
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_AVAILABLE),
    ]);
    let client = QnapClient::new(&fake, "monitor", "secret");
    client.system().await.expect("first");
    client.set_credentials("admin", "secret").await;
    let second = client.system().await.expect("second");
    assert_eq!(second.status.update_available, Some(true));
    assert_eq!(calls(&fake).len(), 10);
}

#[tokio::test]
async fn logging_in_again_fetches_the_firmware_again() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_NONE),
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_AVAILABLE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    client.system().await.expect("first");
    client.login().await.expect("login");
    let second = client.system().await.expect("second");
    assert_eq!(second.status.update_available, Some(true));
}

#[tokio::test]
async fn missing_fields_stay_empty() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(
            "<QDocRoot><authPassed>1</authPassed><func><ownContent><root><cpu_usage>n/a</cpu_usage></root></ownContent></func></QDocRoot>",
        ),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_NONE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");
    assert_eq!(system.model, None);
    assert_eq!(system.firmware, None);
    assert_eq!(system.hostname, None);
    assert_eq!(system.status.cpu_percent, None);
    assert_eq!(system.status.uptime_seconds, None);
    assert_eq!(system.status.memory_total_bytes, None);
    assert!(system.fans.is_empty());
}

#[tokio::test]
async fn an_error_of_sysinfo_is_returned() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Err(ProviderError::Timeout)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.system().await, Err(ProviderError::Timeout));
    assert_eq!(calls(&fake).len(), 2);
}

#[tokio::test]
async fn a_network_error_of_a_part_is_returned() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Err(ProviderError::Unreachable("reset".into())),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(
        client.system().await,
        Err(ProviderError::Unreachable("reset".into()))
    );
}

#[tokio::test]
async fn a_network_error_of_the_update_only_leaves_it_empty() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Err(ProviderError::Timeout),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_AVAILABLE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let first = client.system().await.expect("first");
    assert_eq!(first.model.as_deref(), Some("TS-1685"));
    assert_eq!(first.status.update_available, None);
    // Not cached: the next poll asks again.
    let second = client.system().await.expect("second");
    assert_eq!(second.status.update_available, Some(true));
}

#[tokio::test]
async fn the_firmware_answer_is_reused_within_the_hour() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Ok(FIRMWARE_AVAILABLE),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let first = client.system().await.expect("first");
    let second = client.system().await.expect("second");
    assert_eq!(second.status.update_available, Some(true));
    assert_eq!(second.status.update_version, first.status.update_version);
    let firmware_requests = calls(&fake)
        .iter()
        .filter(|c| c.path == "/cgi-bin/sys/sysRequest.cgi")
        .count();
    assert_eq!(firmware_requests, 1);
    assert_eq!(calls(&fake).len(), 8);
}

#[tokio::test]
async fn a_refused_firmware_answer_is_reused_too() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
        Err(ProviderError::PermissionDenied),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Ok(UPS),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    client.system().await.expect("first");
    let second = client.system().await.expect("second");
    assert_eq!(second.status.update_available, None);
    assert_eq!(calls(&fake).len(), 8);
}

async fn ups_of(answer: Result<&str, ProviderError>) -> Option<Ups> {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        answer,
        Ok(FIRMWARE_NONE),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let system = client.system().await.expect("system");
    // The other values are still there.
    assert_eq!(system.model.as_deref(), Some("TS-1685"));
    assert_eq!(system.status.update_available, Some(false));
    system.ups
}

#[tokio::test]
async fn a_ups_on_battery_is_read() {
    assert_eq!(
        ups_of(Ok(UPS_ON_BATTERY)).await,
        Some(Ups {
            state: Some(UpsState::OnBattery),
            charge_percent: Some(57.0),
            runtime_seconds: Some(12 * 60 + 30),
        })
    );
}

#[tokio::test]
async fn without_a_ups_there_is_none() {
    assert_eq!(ups_of(Ok(UPS_NONE)).await, None);
}

#[tokio::test]
async fn a_failed_ups_page_only_leaves_the_ups_empty() {
    assert_eq!(ups_of(Err(ProviderError::Http(404))).await, None);
    assert_eq!(ups_of(Err(ProviderError::PermissionDenied)).await, None);
    assert_eq!(ups_of(Ok("<html>Not found</html>")).await, None);
}

#[tokio::test]
async fn a_network_error_of_the_ups_page_is_returned() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Ok(SYSINFO),
        Ok(HEALTH_GOOD),
        Err(ProviderError::Timeout),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.system().await, Err(ProviderError::Timeout));
}
