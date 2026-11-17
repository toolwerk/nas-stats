//! Current state of the three demo NAS, values from the mockup.

use nas_model::{
    BackupKind, BackupResult, BackupTask, Container, Disk, Health, Nas, NetInterface, ProviderKind,
    ScrubState, ScrubStatus, Snapshot, SystemStatus, Ups, UpsState, Volume, VolumeKind,
    hash_serial,
};

const GB: u64 = 1_000_000_000;
const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// Keller, Büro and Homelab, in this order, all observed at `now`.
pub(crate) fn snapshots(now: i64) -> Vec<Snapshot> {
    vec![keller(now), buero(now), homelab(now)]
}

fn nas(slug: &str, provider: ProviderKind, name: &str, model: &str, firmware: &str) -> Nas {
    Nas {
        id: format!("demo-{slug}"),
        provider,
        name: Some(name.into()),
        base_url: format!("https://{slug}.demo.invalid"),
        model: Some(model.into()),
        firmware: Some(firmware.into()),
        hostname: Some(format!("{slug}-demo")),
    }
}

struct DiskSpec {
    model: &'static str,
    temperature_c: f64,
    power_on_hours: u64,
    /// An SSD has no spin state.
    ssd: bool,
}

fn disks(nas_id: &str, specs: &[DiskSpec]) -> Vec<Disk> {
    specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let n = i + 1;
            Disk {
                id: format!("{nas_id}-disk{n}"),
                name: format!("Platte {n}"),
                model: Some(spec.model.into()),
                serial_hash: Some(hash_serial(nas_id, &format!("DEMO-{n}"))),
                temperature_c: Some(spec.temperature_c),
                health: Some(Health::Ok),
                smart_text: Some("Gut".into()),
                power_on_hours: Some(spec.power_on_hours),
                spinning: (!spec.ssd).then_some(true),
            }
        })
        .collect()
}

fn containers(names: &[(&str, bool)]) -> Vec<Container> {
    names
        .iter()
        .map(|(name, running)| Container {
            name: (*name).into(),
            running: *running,
        })
        .collect()
}

fn interface(name: &str, rx_bytes_per_sec: f64, tx_bytes_per_sec: f64) -> NetInterface {
    NetInterface {
        name: name.into(),
        rx_bytes_per_sec: Some(rx_bytes_per_sec),
        tx_bytes_per_sec: Some(tx_bytes_per_sec),
    }
}

fn keller(now: i64) -> Snapshot {
    let nas = nas(
        "keller",
        ProviderKind::Qnap,
        "Keller",
        "TS-464",
        "5.2.4.3079 Build 20250321",
    );
    let id = nas.id.clone();
    let pool_id = format!("{id}-pool1");
    Snapshot {
        observed_at: now,
        system: SystemStatus {
            uptime_seconds: Some(38 * DAY as u64),
            cpu_percent: Some(9.0),
            memory_percent: Some(41.0),
            memory_total_bytes: Some(8 * GB),
            temperature_c: Some(41.0),
            cpu_temperature_c: Some(48.0),
            update_available: Some(true),
            update_version: Some("5.2.5.3150 Build 20251020".into()),
        },
        ups: Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: Some(100.0),
            runtime_seconds: Some(34 * MINUTE as u64),
        }),
        volumes: vec![
            Volume {
                id: pool_id.clone(),
                name: "Speicherpool 1".into(),
                kind: VolumeKind::Pool,
                pool_id: None,
                used_bytes: None,
                total_bytes: None,
                health: Some(Health::Ok),
                status_text: Some("Bereit".into()),
                raid: Some("RAID 5".into()),
                scrub: None,
            },
            Volume {
                id: format!("{id}-datavol1"),
                name: "DataVol1".into(),
                kind: VolumeKind::Volume,
                pool_id: Some(pool_id),
                used_bytes: Some(18_990 * GB),
                total_bytes: Some(21_800 * GB),
                health: Some(Health::Ok),
                status_text: Some("Bereit".into()),
                raid: None,
                scrub: None,
            },
        ],
        disks: disks(
            &id,
            &[
                DiskSpec {
                    model: "WD Red Plus 8 TB",
                    temperature_c: 38.0,
                    power_on_hours: 21_412,
                    ssd: false,
                },
                DiskSpec {
                    model: "WD Red Plus 8 TB",
                    temperature_c: 41.0,
                    power_on_hours: 21_409,
                    ssd: false,
                },
                DiskSpec {
                    model: "WD Red Plus 8 TB",
                    temperature_c: 52.0,
                    power_on_hours: 21_410,
                    ssd: false,
                },
                DiskSpec {
                    model: "Seagate IronWolf 8 TB",
                    temperature_c: 39.0,
                    power_on_hours: 3_120,
                    ssd: false,
                },
            ],
        ),
        fans: Vec::new(),
        backups: vec![
            BackupTask {
                id: format!("{id}-backup-offsite"),
                name: "Offsite B2".into(),
                kind: BackupKind::Hbs3,
                last_run: Some(now - 9 * HOUR),
                last_result: Some(BackupResult::Ok),
                next_run: Some(now + 11 * HOUR + 55 * MINUTE),
                interval_secs: None,
            },
            BackupTask {
                id: format!("{id}-backup-snapshots"),
                name: "Snapshots DataVol1".into(),
                kind: BackupKind::Snapshot,
                last_run: Some(now - 5 * MINUTE),
                last_result: Some(BackupResult::Ok),
                next_run: Some(now + 55 * MINUTE),
                interval_secs: None,
            },
        ],
        containers: containers(&[
            ("Paperless", true),
            ("Jellyfin", true),
            ("Immich", true),
            ("Vaultwarden", true),
            ("Uptime Kuma", true),
            ("Mosquitto", true),
            ("Watchtower", false),
        ]),
        alerts: Vec::new(),
        interfaces: vec![interface("Adapter 1", 4_200_000.0, 300_000.0)],
        nas,
    }
}

fn buero(now: i64) -> Snapshot {
    let nas = nas(
        "buero",
        ProviderKind::Synology,
        "Büro",
        "DS923+",
        "DSM 7.3.1",
    );
    let id = nas.id.clone();
    Snapshot {
        observed_at: now,
        system: SystemStatus {
            uptime_seconds: Some(12 * DAY as u64),
            cpu_percent: Some(4.0),
            memory_percent: Some(33.0),
            memory_total_bytes: Some(4 * GB),
            temperature_c: Some(42.0),
            cpu_temperature_c: None,
            update_available: Some(false),
            update_version: None,
        },
        ups: None,
        volumes: vec![Volume {
            id: format!("{id}-volume1"),
            name: "Volume 1".into(),
            kind: VolumeKind::Volume,
            pool_id: None,
            used_bytes: Some(5_670 * GB),
            total_bytes: Some(10_900 * GB),
            health: Some(Health::Ok),
            status_text: Some("Normal".into()),
            raid: Some("SHR".into()),
            scrub: None,
        }],
        disks: disks(
            &id,
            &[
                DiskSpec {
                    model: "Synology HAT3300 4 TB",
                    temperature_c: 36.0,
                    power_on_hours: 9_204,
                    ssd: false,
                },
                DiskSpec {
                    model: "Synology HAT3300 4 TB",
                    temperature_c: 37.0,
                    power_on_hours: 9_204,
                    ssd: false,
                },
                DiskSpec {
                    model: "Synology HAT3300 4 TB",
                    temperature_c: 39.0,
                    power_on_hours: 9_203,
                    ssd: false,
                },
                DiskSpec {
                    model: "Synology HAT3300 4 TB",
                    temperature_c: 38.0,
                    power_on_hours: 9_203,
                    ssd: false,
                },
            ],
        ),
        fans: Vec::new(),
        backups: vec![BackupTask {
            id: format!("{id}-backup-c2"),
            name: "Cloud C2".into(),
            kind: BackupKind::HyperBackup,
            last_run: Some(now - 3 * HOUR),
            last_result: Some(BackupResult::Ok),
            next_run: Some(now + 21 * HOUR),
            interval_secs: None,
        }],
        containers: Vec::new(),
        alerts: Vec::new(),
        interfaces: vec![interface("LAN 1", 1_100_000.0, 200_000.0)],
        nas,
    }
}

fn homelab(now: i64) -> Snapshot {
    let nas = nas(
        "homelab",
        ProviderKind::TrueNas,
        "Homelab",
        "TrueNAS CE 25.10",
        "25.10.1",
    );
    let id = nas.id.clone();
    Snapshot {
        observed_at: now,
        system: SystemStatus {
            uptime_seconds: Some(61 * DAY as u64),
            cpu_percent: Some(17.0),
            memory_percent: Some(71.0),
            memory_total_bytes: Some(32 * GB),
            temperature_c: Some(46.0),
            cpu_temperature_c: Some(52.0),
            update_available: Some(false),
            update_version: None,
        },
        ups: Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: Some(100.0),
            runtime_seconds: Some(52 * MINUTE as u64),
        }),
        volumes: vec![
            Volume {
                id: format!("{id}-tank"),
                name: "tank".into(),
                kind: VolumeKind::Pool,
                pool_id: None,
                used_bytes: Some(13_820 * GB),
                total_bytes: Some(21_600 * GB),
                health: Some(Health::Ok),
                status_text: Some("ONLINE".into()),
                raid: Some("raidz2".into()),
                scrub: Some(ScrubStatus {
                    state: Some(ScrubState::Finished),
                    last_finished_at: Some(now - 6 * DAY),
                    errors: Some(0),
                    progress_percent: None,
                }),
            },
            Volume {
                id: format!("{id}-apps"),
                name: "apps".into(),
                kind: VolumeKind::Pool,
                pool_id: None,
                used_bytes: Some(210 * GB),
                total_bytes: Some(900 * GB),
                health: Some(Health::Ok),
                status_text: Some("ONLINE".into()),
                raid: Some("mirror".into()),
                scrub: None,
            },
        ],
        disks: disks(
            &id,
            &[
                DiskSpec {
                    model: "Seagate IronWolf Pro 12 TB",
                    temperature_c: 40.0,
                    power_on_hours: 14_870,
                    ssd: false,
                },
                DiskSpec {
                    model: "Seagate IronWolf Pro 12 TB",
                    temperature_c: 42.0,
                    power_on_hours: 14_869,
                    ssd: false,
                },
                DiskSpec {
                    model: "Seagate IronWolf Pro 12 TB",
                    temperature_c: 44.0,
                    power_on_hours: 14_871,
                    ssd: false,
                },
                DiskSpec {
                    model: "Seagate IronWolf Pro 12 TB",
                    temperature_c: 41.0,
                    power_on_hours: 14_868,
                    ssd: false,
                },
                DiskSpec {
                    model: "Samsung 870 EVO 1 TB",
                    temperature_c: 33.0,
                    power_on_hours: 6_402,
                    ssd: true,
                },
                DiskSpec {
                    model: "Samsung 870 EVO 1 TB",
                    temperature_c: 34.0,
                    power_on_hours: 6_402,
                    ssd: true,
                },
            ],
        ),
        fans: Vec::new(),
        backups: vec![BackupTask {
            id: format!("{id}-backup-replication"),
            name: "Replikation tank".into(),
            kind: BackupKind::Replication,
            last_run: Some(now - HOUR),
            last_result: Some(BackupResult::Ok),
            next_run: Some(now + 23 * HOUR),
            interval_secs: None,
        }],
        containers: containers(&[
            ("Home Assistant", true),
            ("Grafana", true),
            ("Prometheus", true),
            ("Pi-hole", true),
            ("Syncthing", true),
        ]),
        alerts: Vec::new(),
        interfaces: vec![interface("enp3s0", 2_400_000.0, 1_100_000.0)],
        nas,
    }
}
