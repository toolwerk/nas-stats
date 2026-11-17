//! The Unraid snapshot and parity history from answers built after the
//! schema of the Unraid API 4.25 (see `fixtures/README.md`).

mod common;

use common::{DOCKER, FakeTransport, MAIN, METRICS, NOTIFICATIONS, PARITY_HISTORY, SHARES, UPS};
use nas_model::{
    Alert, AlertLevel, BackupKind, BackupResult, BackupRun, Container, Health, Provider,
    ProviderError, ProviderKind, ScrubState, Ups, UpsState, VolumeKind, hash_serial,
};
use provider_unraid::{PARITY_TASK_ID, UnraidClient, UnraidProvider};

const MAIN_FULL: &str = include_str!("fixtures/main.json");
const MAIN_MINIMAL: &str = include_str!("fixtures/main_minimal.json");
const MAIN_PARITY_RUNNING: &str = include_str!("fixtures/main_parity_running.json");
const METRICS_OK: &str = include_str!("fixtures/metrics.json");
const SHARES_OK: &str = include_str!("fixtures/shares.json");
const NOTIFICATIONS_OK: &str = include_str!("fixtures/notifications.json");
const UPS_OK: &str = include_str!("fixtures/ups.json");
const UPS_NONE: &str = include_str!("fixtures/ups_none.json");
const UPS_NO_APCUPSD: &str = include_str!("fixtures/ups_no_apcupsd.json");
const DOCKER_OK: &str = include_str!("fixtures/docker.json");
const DOCKER_DISABLED: &str = include_str!("fixtures/docker_disabled.json");
const UNAUTHENTICATED: &str = include_str!("fixtures/unauthenticated.json");
const FORBIDDEN: &str = include_str!("fixtures/forbidden.json");
const HISTORY: &str = include_str!("fixtures/parity_history.json");
const HISTORY_EMPTY: &str = include_str!("fixtures/parity_history_empty.json");
const HISTORY_MISSING: &str = include_str!("fixtures/parity_history_missing.json");

const BASE_URL: &str = "https://tower.example:443";
const KEY: &str = "demo-key";

/// 2026-11-01T01:00:00Z, start of the last parity check in `main.json`.
const PARITY_START: i64 = 1_793_494_800;
/// 2026-11-01T06:00:00Z, boot time in `main.json`.
const BOOT: i64 = 1_793_512_800;

fn provider(fake: &FakeTransport) -> UnraidProvider<&FakeTransport> {
    UnraidProvider::new("nas-1", BASE_URL, UnraidClient::new(fake, KEY))
}

/// Every query answered from the full fixtures, with `main` for the
/// required query.
fn all_ok(main: &'static str) -> Vec<(&'static str, Result<&'static str, ProviderError>)> {
    vec![
        (MAIN, Ok(main)),
        (METRICS, Ok(METRICS_OK)),
        (SHARES, Ok(SHARES_OK)),
        (NOTIFICATIONS, Ok(NOTIFICATIONS_OK)),
        (UPS, Ok(UPS_OK)),
        (DOCKER, Ok(DOCKER_OK)),
    ]
}

/// `answers` with the answer for `field` replaced.
fn with(
    mut answers: Vec<(&'static str, Result<&'static str, ProviderError>)>,
    field: &'static str,
    answer: Result<&'static str, ProviderError>,
) -> Vec<(&'static str, Result<&'static str, ProviderError>)> {
    for entry in &mut answers {
        if entry.0 == field {
            entry.1 = answer.clone();
        }
    }
    answers
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
}

#[tokio::test]
async fn snapshot_holds_every_part() {
    let fake = FakeTransport::new(all_ok(MAIN_FULL));
    let provider = provider(&fake);
    assert_eq!(provider.kind(), ProviderKind::Unraid);

    let before = now();
    let snapshot = provider.snapshot().await.expect("snapshot");
    let after = now();

    // One query each, one after the other, every one with the key.
    assert_eq!(
        fake.fields(),
        vec![MAIN, METRICS, SHARES, NOTIFICATIONS, UPS, DOCKER]
    );
    assert!(fake.calls().iter().all(|call| call.api_key == KEY));

    let nas = &snapshot.nas;
    assert_eq!(nas.id, "nas-1");
    assert_eq!(nas.provider, ProviderKind::Unraid);
    assert_eq!(nas.base_url, BASE_URL);
    assert_eq!(nas.name.as_deref(), Some("Tower"));
    assert_eq!(nas.hostname.as_deref(), Some("tower-demo"));
    assert_eq!(nas.model.as_deref(), Some("Demo Board Inc. B550-DEMO"));
    assert_eq!(nas.firmware.as_deref(), Some("7.2.0"));
    assert!((before..=after).contains(&snapshot.observed_at));

    let system = &snapshot.system;
    let uptime = system.uptime_seconds.expect("uptime") as i64;
    assert!((before - BOOT..=after - BOOT).contains(&uptime), "{uptime}");
    assert_eq!(system.cpu_percent, Some(7.5));
    assert_eq!(system.memory_percent, Some(41.25));
    assert_eq!(system.memory_total_bytes, Some(33_554_432_000));
    assert_eq!(system.temperature_c, None);
    assert_eq!(system.cpu_temperature_c, None);
    assert_eq!(system.update_available, None);

    // Array first, then the shares; kilobytes of 1000 bytes.
    let volumes = &snapshot.volumes;
    assert_eq!(volumes.len(), 4);
    let array = &volumes[0];
    assert_eq!(array.id, "array");
    assert_eq!(array.kind, VolumeKind::Array);
    assert_eq!(array.used_bytes, Some(2_000_000_000_000));
    assert_eq!(array.total_bytes, Some(8_000_000_000_000));
    assert_eq!(array.health, Some(Health::Ok));
    assert_eq!(array.status_text.as_deref(), Some("STARTED"));
    let scrub = array.scrub.as_ref().expect("scrub");
    assert_eq!(scrub.state, Some(ScrubState::Finished));
    assert_eq!(scrub.last_finished_at, Some(PARITY_START + 36_000));
    assert_eq!(scrub.progress_percent, None);
    assert_eq!(scrub.errors, None);

    let shares: Vec<(&str, &str, Option<u64>, Option<u64>)> = volumes[1..]
        .iter()
        .map(|v| {
            assert_eq!(v.kind, VolumeKind::Share);
            (v.id.as_str(), v.name.as_str(), v.used_bytes, v.total_bytes)
        })
        .collect();
    assert_eq!(
        shares,
        vec![
            (
                "share:appdata",
                "appdata",
                Some(25_000_000_000),
                Some(525_000_000_000)
            ),
            (
                "share:share-1",
                "share-1",
                Some(1_900_000_000_000),
                Some(7_900_000_000_000)
            ),
            ("share:share-2", "share-2", Some(1_000_000), None),
        ]
    );

    // Parity, data and cache disks, without empty slots.
    let disks = &snapshot.disks;
    let names: Vec<&str> = disks.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(names, vec!["parity", "disk1", "disk2", "disk3", "cache"]);
    let parity = &disks[0];
    assert_eq!(parity.name, "parity");
    assert_eq!(parity.temperature_c, Some(33.0));
    assert_eq!(parity.health, Some(Health::Ok));
    assert_eq!(parity.spinning, Some(true));
    assert_eq!(
        parity.serial_hash,
        Some(hash_serial("nas-1", "MODEL_SERIAL-A"))
    );
    assert_eq!(parity.model, None);
    assert_eq!(parity.smart_text, None);
    assert_eq!(parity.power_on_hours, None);
    // Sleeping disk with read errors: no temperature, a warning.
    assert_eq!(disks[2].temperature_c, None);
    assert_eq!(disks[2].spinning, Some(false));
    assert_eq!(disks[2].health, Some(Health::Warning));
    assert_eq!(disks[3].health, Some(Health::Critical));
    // The cache id is only its device: no hash.
    assert_eq!(disks[4].serial_hash, None);
    // The plain id is kept nowhere.
    let json = serde_json::to_string(&snapshot).expect("json");
    assert!(!json.contains("SERIAL"), "{json}");

    assert_eq!(snapshot.backups.len(), 1);
    let task = &snapshot.backups[0];
    assert_eq!(task.id, PARITY_TASK_ID);
    assert_eq!(task.name, "Paritätsprüfung");
    assert_eq!(task.kind, BackupKind::Parity);
    assert_eq!(task.last_run, Some(PARITY_START));
    assert_eq!(task.last_result, Some(BackupResult::Ok));
    assert_eq!(task.next_run, None);

    assert_eq!(
        snapshot.alerts,
        vec![
            Alert {
                level: AlertLevel::Info,
                text: "Parity disk returned to normal operation".into(),
                since: Some(1_794_816_000),
            },
            Alert {
                level: AlertLevel::Warning,
                text: "Warning [TOWER-DEMO] - disk3 has read errors".into(),
                since: Some(1_794_819_600),
            },
            Alert {
                level: AlertLevel::Critical,
                text: "Array has 1 disk with errors".into(),
                since: None,
            },
        ]
    );

    assert_eq!(
        snapshot.ups,
        Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: Some(98.0),
            runtime_seconds: Some(2_700),
        })
    );

    assert_eq!(
        snapshot.containers,
        vec![
            Container {
                name: "container-1".into(),
                running: true,
            },
            Container {
                name: "container-2".into(),
                running: false,
            },
            Container {
                name: "7e4f0aa1b2c5".into(),
                running: false,
            },
        ]
    );
    assert!(snapshot.fans.is_empty());
    assert!(snapshot.interfaces.is_empty());
    assert_eq!(snapshot.worst_health(), Some(Health::Critical));
}

#[tokio::test]
async fn missing_fields_stay_empty() {
    let fake = FakeTransport::new(all_ok(MAIN_MINIMAL));
    let snapshot = provider(&fake).snapshot().await.expect("snapshot");

    assert_eq!(snapshot.nas.name, None);
    assert_eq!(snapshot.nas.model, None);
    assert_eq!(snapshot.nas.firmware, None);
    assert_eq!(snapshot.nas.hostname, None);
    // Boot time in the future: no uptime.
    assert_eq!(snapshot.system.uptime_seconds, None);

    // A stopped array reports no space; state compared without case.
    let array = &snapshot.volumes[0];
    assert_eq!(array.health, Some(Health::Warning));
    assert_eq!(array.status_text.as_deref(), Some("stopped"));
    assert_eq!(array.used_bytes, None);
    assert_eq!(array.total_bytes, None);
    assert_eq!(array.scrub, None);

    // A disk without name and a broken entry are skipped; numbers and
    // flags as text are read.
    let disks = &snapshot.disks;
    assert_eq!(disks.len(), 2);
    assert_eq!(disks[1].name, "disk1");
    assert_eq!(disks[1].temperature_c, Some(36.0));
    assert_eq!(disks[1].spinning, Some(true));
    assert_eq!(disks[1].health, None);
    assert_eq!(disks[1].serial_hash, None);

    let task = &snapshot.backups[0];
    assert_eq!(task.last_run, None);
    assert_eq!(task.last_result, Some(BackupResult::Never));
}

#[tokio::test]
async fn running_parity_check_shows_progress_and_no_task_without_parity() {
    let fake = FakeTransport::new(all_ok(MAIN_PARITY_RUNNING));
    let snapshot = provider(&fake).snapshot().await.expect("snapshot");
    let scrub = snapshot.volumes[0].scrub.as_ref().expect("scrub");
    assert_eq!(scrub.state, Some(ScrubState::Running));
    assert_eq!(scrub.progress_percent, Some(12.0));
    assert_eq!(scrub.last_finished_at, None);
    // Numbers as JSON numbers work as well.
    assert_eq!(snapshot.volumes[0].total_bytes, Some(8_000_000_000_000));
    // No parity disk: no parity task.
    assert!(snapshot.backups.is_empty());
    assert!(snapshot.disks.is_empty());
}

#[tokio::test]
async fn refused_key_fails_the_snapshot_as_login_failed() {
    let fake = FakeTransport::new(vec![(MAIN, Ok(UNAUTHENTICATED))]);
    let err = provider(&fake).snapshot().await.expect_err("refused");
    assert_eq!(err, ProviderError::LoginFailed);
    assert!(err.stops_polling());
    assert_eq!(fake.fields(), vec![MAIN]);

    let fake = FakeTransport::new(vec![(MAIN, Err(ProviderError::LoginFailed))]);
    let err = provider(&fake).snapshot().await.expect_err("HTTP 401");
    assert_eq!(err, ProviderError::LoginFailed);
}

#[tokio::test]
async fn refused_key_in_an_optional_query_fails_the_snapshot() {
    let answers = with(all_ok(MAIN_FULL), METRICS, Ok(UNAUTHENTICATED));
    let fake = FakeTransport::new(answers);
    let err = provider(&fake).snapshot().await.expect_err("refused");
    assert_eq!(err, ProviderError::LoginFailed);
    assert_eq!(fake.fields(), vec![MAIN, METRICS]);
}

#[tokio::test]
async fn missing_docker_ups_and_rights_leave_only_that_part_empty() {
    let answers = with(all_ok(MAIN_FULL), DOCKER, Ok(DOCKER_DISABLED));
    let answers = with(answers, UPS, Ok(UPS_NONE));
    let answers = with(answers, NOTIFICATIONS, Ok(FORBIDDEN));
    let answers = with(answers, SHARES, Err(ProviderError::Http(500)));
    let answers = with(answers, METRICS, Err(ProviderError::PermissionDenied));
    let fake = FakeTransport::new(answers);
    let snapshot = provider(&fake).snapshot().await.expect("snapshot");
    assert!(snapshot.containers.is_empty());
    assert_eq!(snapshot.ups, None);
    assert!(snapshot.alerts.is_empty());
    assert_eq!(snapshot.volumes.len(), 1);
    assert_eq!(snapshot.system.cpu_percent, None);
    // The rest is there.
    assert_eq!(snapshot.disks.len(), 5);
    assert_eq!(snapshot.backups.len(), 1);
    assert_eq!(snapshot.nas.name.as_deref(), Some("Tower"));
}

#[tokio::test]
async fn network_errors_of_optional_queries_fail_the_snapshot() {
    for error in [
        ProviderError::Timeout,
        ProviderError::Unreachable("weg".into()),
        ProviderError::CertificateChanged {
            old: "aa".into(),
            new: "bb".into(),
        },
    ] {
        let answers = with(all_ok(MAIN_FULL), DOCKER, Err(error.clone()));
        let fake = FakeTransport::new(answers);
        let err = provider(&fake).snapshot().await.expect_err("fails");
        assert_eq!(err, error);
    }
}

#[tokio::test]
async fn errors_of_the_main_query_fail_the_snapshot() {
    let fake = FakeTransport::new(vec![(MAIN, Ok(DOCKER_DISABLED))]);
    let err = provider(&fake).snapshot().await.expect_err("fails");
    assert!(matches!(err, ProviderError::Parse(_)), "{err:?}");
    let fake = FakeTransport::new(vec![(MAIN, Ok(FORBIDDEN))]);
    let err = provider(&fake).snapshot().await.expect_err("fails");
    assert_eq!(err, ProviderError::PermissionDenied);
    let fake = FakeTransport::new(vec![(MAIN, Ok("<html>"))]);
    let err = provider(&fake).snapshot().await.expect_err("fails");
    assert!(matches!(err, ProviderError::Parse(_)), "{err:?}");
}

#[tokio::test]
async fn new_api_key_is_sent_with_the_next_query() {
    let fake = FakeTransport::new(all_ok(MAIN_FULL));
    let provider = provider(&fake);
    provider.client().set_api_key("neu").await;
    provider.snapshot().await.expect("snapshot");
    assert!(fake.calls().iter().all(|call| call.api_key == "neu"));
    // The key is not in the debug output.
    assert!(!format!("{:?}", provider.client()).contains("neu"));
}

#[tokio::test]
async fn parity_history_becomes_backup_runs_since() {
    let fake = FakeTransport::new(vec![(PARITY_HISTORY, Ok(HISTORY))]);
    let runs = provider(&fake)
        .backup_runs(1_790_816_400)
        .await
        .expect("runs");
    assert_eq!(fake.fields(), vec![PARITY_HISTORY]);
    let run = |started_at, finished_at, result, message: Option<&str>| BackupRun {
        task_id: PARITY_TASK_ID.into(),
        started_at,
        finished_at: Some(finished_at),
        result,
        message: message.map(str::to_owned),
    };
    assert_eq!(
        runs,
        vec![
            run(1_790_816_400, 1_790_852_400, BackupResult::Ok, None),
            run(
                1_792_033_200,
                1_792_040_400,
                BackupResult::Error,
                Some("abgebrochen")
            ),
            run(
                1_792_458_000,
                1_792_494_000,
                BackupResult::Error,
                Some("12 Fehler")
            ),
            run(1_793_494_800, 1_793_530_800, BackupResult::Ok, None),
        ]
    );
}

#[tokio::test]
async fn empty_or_missing_parity_history_has_no_runs() {
    let fake = FakeTransport::new(vec![(PARITY_HISTORY, Ok(HISTORY_EMPTY))]);
    assert_eq!(provider(&fake).backup_runs(0).await, Ok(Vec::new()));
    let fake = FakeTransport::new(vec![(PARITY_HISTORY, Ok(HISTORY_MISSING))]);
    assert_eq!(provider(&fake).backup_runs(0).await, Ok(Vec::new()));
    let fake = FakeTransport::new(vec![(PARITY_HISTORY, Ok(UNAUTHENTICATED))]);
    assert_eq!(
        provider(&fake).backup_runs(0).await,
        Err(ProviderError::LoginFailed)
    );
}

#[tokio::test]
async fn queries_never_ask_for_disks_or_the_docker_organizer() {
    let fake = FakeTransport::new(all_ok(MAIN_FULL));
    provider(&fake).snapshot().await.expect("snapshot");
    for call in fake.calls() {
        // `disks` only inside `array { … }`, never as a root field.
        assert!(!call.query.starts_with("query { disks"), "{}", call.query);
        assert!(!call.query.contains("organizer"), "{}", call.query);
        assert!(!call.query.contains("smart"), "{}", call.query);
    }
}

#[tokio::test]
async fn apcupsd_states_map_to_online_and_on_battery() {
    let answer = |status: &str| {
        format!(
            r#"{{"data":{{"upsDevices":[{{"status":"{status}","battery":{{"chargeLevel":50,"estimatedRuntime":"10"}}}}]}}}}"#
        )
    };
    for (status, state) in [
        ("ONLINE", Some(UpsState::Online)),
        ("TRIM ONLINE", Some(UpsState::Online)),
        ("BOOST ONLINE", Some(UpsState::Online)),
        ("ONBATT", Some(UpsState::OnBattery)),
        ("ONBATT LOWBATT", Some(UpsState::OnBattery)),
        ("OL CHRG", Some(UpsState::Online)),
        ("OB DISCHRG", Some(UpsState::OnBattery)),
        ("ONLINE LOWBATT", Some(UpsState::Online)),
        ("OL CHRG LB", Some(UpsState::Online)),
        ("LOWBATT", Some(UpsState::OnBattery)),
        ("COMMLOST", None),
        ("", None),
    ] {
        let body: &'static str = Box::leak(answer(status).into_boxed_str());
        let fake = FakeTransport::new(with(all_ok(MAIN_FULL), UPS, Ok(body)));
        let snapshot = provider(&fake).snapshot().await.expect("snapshot");
        let ups = snapshot.ups.expect("ups");
        assert_eq!(ups.state, state, "{status}");
        assert_eq!(ups.runtime_seconds, Some(600));
    }
}

#[tokio::test]
async fn completed_parity_check_without_date_keeps_its_state() {
    let main = MAIN_FULL.replace(r#""date": "2026-11-01T01:00:00.000Z""#, r#""date": null"#);
    let main: &'static str = Box::leak(main.into_boxed_str());
    let fake = FakeTransport::new(all_ok(main));
    let snapshot = provider(&fake).snapshot().await.expect("snapshot");
    let scrub = snapshot.volumes[0].scrub.as_ref().expect("scrub");
    assert_eq!(scrub.state, Some(ScrubState::Finished));
    assert_eq!(scrub.last_finished_at, None);
    assert_eq!(snapshot.backups[0].last_run, None);
}

#[tokio::test]
async fn without_apcupsd_the_ups_stays_empty() {
    let answers = with(all_ok(MAIN_FULL), UPS, Ok(UPS_NO_APCUPSD));
    let fake = FakeTransport::new(answers);
    let snapshot = provider(&fake).snapshot().await.expect("snapshot");
    assert_eq!(snapshot.ups, None);
    assert_eq!(snapshot.containers.len(), 3);
}
