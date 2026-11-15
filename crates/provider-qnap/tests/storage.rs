//! Volumes and disks from the recorded answers (issue #1) and from rebuilt
//! answers with a volume and a disk in warning state.

mod common;

use common::{FakeTransport, calls, param};
use nas_model::{Health, ProviderError, VolumeKind, hash_serial};
use provider_qnap::QnapClient;

const LOGIN_OK: &str = include_str!("fixtures/login.xml");
const USAGE: &str = include_str!("fixtures/disk_usage.xml");
const SMART: &str = include_str!("fixtures/qsmart.xml");
const USAGE_WARNING: &str = include_str!("fixtures/disk_usage_warning.xml");
const SMART_WARNING: &str = include_str!("fixtures/qsmart_warning.xml");

#[tokio::test]
async fn reads_the_recorded_volumes_and_disks() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(USAGE), Ok(SMART)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let storage = client.storage("nas-1").await.expect("storage");

    assert_eq!(storage.volumes.len(), 3);
    let v1 = &storage.volumes[0];
    assert_eq!(v1.id, "2");
    assert_eq!(v1.name, "Volume 1");
    assert_eq!(v1.kind, VolumeKind::Volume);
    assert_eq!(v1.pool_id, None);
    assert_eq!(v1.raid.as_deref(), Some("raid5"));
    assert_eq!(v1.total_bytes, Some(10_949_742_198_784));
    assert_eq!(v1.used_bytes, Some(10_949_742_198_784 - 651_822_833_664));
    // Code 44 is the Warning QTS shows for a volume that is over 94 % full.
    assert_eq!(v1.status_text.as_deref(), Some("44"));
    assert_eq!(v1.health, Some(Health::Warning));
    assert!(v1.used_percent().expect("percent") > 94.0);
    let v3 = &storage.volumes[2];
    assert_eq!((v3.id.as_str(), v3.name.as_str()), ("1", "Volume 3"));
    assert_eq!(v3.health, Some(Health::Ok));

    let ids: Vec<&str> = storage.disks.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids.len(), 9);
    assert!(ids.iter().all(|id| id.starts_with("0:")));
    let d = storage.disks.iter().find(|d| d.id == "0:11").expect("disk");
    assert_eq!(d.name, "Platte 11");
    assert_eq!(d.model.as_deref(), Some("ST12000NE0008-2PK103"));
    assert_eq!(d.temperature_c, Some(48.0));
    assert_eq!(d.health, Some(Health::Ok));
    assert_eq!(d.smart_text.as_deref(), Some("OK"));
    assert_eq!(d.serial_hash, Some(hash_serial("nas-1", "SERIAL-F")));
    assert_eq!((d.power_on_hours, d.spinning), (None, None));

    let calls = calls(&fake);
    assert_eq!(calls[1].path, "/cgi-bin/management/chartReq.cgi");
    assert_eq!(param(&calls[1], "chart_func"), Some("disk_usage"));
    assert_eq!(param(&calls[1], "disk_select"), Some("all"));
    assert_eq!(param(&calls[1], "include"), Some("all"));
    assert_eq!(calls[2].path, "/cgi-bin/disk/qsmart.cgi");
    assert_eq!(param(&calls[2], "func"), Some("all_hd_data"));
}

#[tokio::test]
async fn reads_a_volume_and_a_disk_in_warning_state() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(USAGE_WARNING), Ok(SMART_WARNING)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let storage = client.storage("nas-1").await.expect("storage");

    // The app derives its warning from these values; QTS reports no state.
    let volume = &storage.volumes[0];
    assert_eq!(volume.name, "DataVol1");
    let percent = volume.used_percent().expect("percent");
    assert!((percent - 87.1).abs() < 0.05, "{percent}");

    // The empty slot 5 is not a disk.
    assert_eq!(storage.disks.len(), 4);
    let disk = &storage.disks[2];
    assert_eq!(disk.name, "Platte 3");
    assert_eq!(disk.temperature_c, Some(52.0));
    assert_eq!(disk.health, Some(Health::Warning));
    assert_eq!(disk.serial_hash, Some(hash_serial("nas-1", "SERIAL-C")));
    // An SMART text we do not know stays without health, the text is kept.
    assert_eq!(storage.disks[3].health, None);
    assert_eq!(storage.disks[3].smart_text.as_deref(), Some("Unbekannt"));
}

#[tokio::test]
async fn an_account_without_rights_gets_empty_lists() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Err(ProviderError::PermissionDenied),
        Err(ProviderError::PermissionDenied),
    ]);
    let client = QnapClient::new(&fake, "user", "secret");
    let storage = client.storage("nas-1").await.expect("storage");
    assert!(storage.volumes.is_empty());
    assert!(storage.disks.is_empty());
}

#[tokio::test]
async fn other_errors_are_returned() {
    let fake = FakeTransport::new(vec![
        Ok(LOGIN_OK),
        Err(ProviderError::Unreachable("timeout".into())),
    ]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert!(matches!(
        client.storage("nas-1").await,
        Err(ProviderError::Unreachable(_))
    ));
}
