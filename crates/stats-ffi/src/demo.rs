//! A fully filled demo snapshot, marked as demo, for the Swift test and
//! for checking the records. Invented values only; the demo data set of the
//! app (#22) is separate.

use std::time::{SystemTime, UNIX_EPOCH};

use nas_model::{
    Alert, AlertLevel, BackupKind, BackupResult, BackupTask, Container, Disk, Fan, Health, Nas,
    NetInterface, ProviderKind, ScrubState, ScrubStatus, Snapshot, SystemStatus, Ups, UpsState,
    Volume, VolumeKind, hash_serial,
};

const NAS_ID: &str = "demo-keller";
const TB: u64 = 1_000_000_000_000;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

fn disk(number: u32, temperature_c: f64, health: Health, smart_text: &str) -> Disk {
    Disk {
        id: format!("disk-{number}"),
        name: format!("Platte {number}"),
        model: Some("Demo HDD 8TB".into()),
        serial_hash: Some(hash_serial(NAS_ID, &format!("DEMO-SERIAL-{number}"))),
        temperature_c: Some(temperature_c),
        health: Some(health),
        smart_text: Some(smart_text.into()),
        power_on_hours: Some(18_000 + u64::from(number) * 100),
        spinning: Some(true),
    }
}

/// The demo NAS "Keller": a QNAP TS-464 with QTS 5.2.4, `DataVol1` 87.1 %
/// full and disk 3 at 52 °C. Every optional field is set and every list
/// holds at least one entry.
pub(crate) fn snapshot() -> Snapshot {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    Snapshot {
        nas: Nas {
            id: NAS_ID.into(),
            provider: ProviderKind::Qnap,
            name: Some("Keller".into()),
            base_url: "https://keller.demo.invalid:443".into(),
            model: Some("TS-464".into()),
            firmware: Some("QTS 5.2.4".into()),
            hostname: Some("keller".into()),
        },
        observed_at: now,
        system: SystemStatus {
            uptime_seconds: Some(12 * 86_400 + 3 * 3_600),
            cpu_percent: Some(18.5),
            memory_percent: Some(42.0),
            memory_total_bytes: Some(16 * 1_073_741_824),
            temperature_c: Some(41.0),
            cpu_temperature_c: Some(56.0),
            update_available: Some(true),
            update_version: Some("QTS 5.2.5".into()),
        },
        ups: Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: Some(100.0),
            runtime_seconds: Some(1_800),
        }),
        volumes: vec![
            Volume {
                id: "pool-1".into(),
                name: "Speicherpool 1".into(),
                kind: VolumeKind::Pool,
                pool_id: Some("pool-1".into()),
                used_bytes: Some(15 * TB),
                total_bytes: Some(21 * TB),
                health: Some(Health::Ok),
                status_text: Some("Bereit".into()),
                raid: Some("RAID 5".into()),
                scrub: Some(ScrubStatus {
                    state: Some(ScrubState::Running),
                    last_finished_at: Some(now - 30 * DAY),
                    errors: Some(0),
                    progress_percent: Some(42.0),
                }),
            },
            Volume {
                id: "vol-1".into(),
                name: "DataVol1".into(),
                kind: VolumeKind::Volume,
                pool_id: Some("pool-1".into()),
                // 87.1 % of 16 TB.
                used_bytes: Some(13_936_000_000_000),
                total_bytes: Some(16 * TB),
                health: Some(Health::Warning),
                status_text: Some("Fast voll".into()),
                raid: Some("RAID 5".into()),
                scrub: Some(ScrubStatus {
                    state: Some(ScrubState::Finished),
                    last_finished_at: Some(now - 2 * DAY),
                    errors: Some(0),
                    progress_percent: Some(100.0),
                }),
            },
        ],
        disks: vec![
            disk(1, 38.0, Health::Ok, "Gut"),
            disk(2, 39.0, Health::Ok, "Gut"),
            disk(3, 52.0, Health::Warning, "Warnung"),
            disk(4, 37.0, Health::Ok, "Gut"),
        ],
        fans: vec![Fan {
            name: "Lüfter 1".into(),
            rpm: Some(1_020),
            health: Some(Health::Ok),
        }],
        backups: vec![BackupTask {
            id: "hbs-1".into(),
            name: "Nachtsicherung".into(),
            kind: BackupKind::Hbs3,
            last_run: Some(now - 8 * HOUR),
            last_result: Some(BackupResult::Ok),
            next_run: Some(now + 16 * HOUR),
            interval_secs: Some(24 * HOUR),
        }],
        containers: vec![Container {
            name: "demo-app".into(),
            running: true,
        }],
        alerts: vec![Alert {
            level: AlertLevel::Warning,
            text: "DataVol1 ist zu 87 % belegt (Demo)".into(),
            since: Some(now - DAY),
        }],
        interfaces: vec![NetInterface {
            name: "Adapter 1".into(),
            rx_bytes_per_sec: Some(1_250_000.0),
            tx_bytes_per_sec: Some(310_000.0),
        }],
    }
}
