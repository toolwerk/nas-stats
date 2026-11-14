//! Everything known about one NAS at one point in time.

use serde::{Deserialize, Serialize};

use crate::backup::{BackupResult, BackupTask};
use crate::device::{Alert, AlertLevel, Container, Fan, NetInterface, Ups, UpsState};
use crate::health::Health;
use crate::nas::{Nas, SystemStatus};
use crate::storage::{Disk, Volume};

/// State of one NAS from a single run over all endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub nas: Nas,
    /// When the values were read, Unix seconds UTC.
    pub observed_at: i64,
    pub system: SystemStatus,
    /// `None` when no UPS is attached or the vendor does not report it.
    pub ups: Option<Ups>,
    pub volumes: Vec<Volume>,
    pub disks: Vec<Disk>,
    pub fans: Vec<Fan>,
    pub backups: Vec<BackupTask>,
    pub containers: Vec<Container>,
    pub alerts: Vec<Alert>,
    pub interfaces: Vec<NetInterface>,
}

impl Snapshot {
    /// Worst health the NAS reports itself, `None` when nothing is known.
    ///
    /// Counts the health of volumes, disks and fans, alerts (`Warning` and
    /// `Critical`; `Info` does not count), backup results (`Ok` is `Ok`,
    /// `Error` is `Warning`, other results do not count) and the UPS
    /// (`Online` is `Ok`, `OnBattery` is `Warning`). Thresholds such as
    /// "disk too hot" are not applied here; the core checks them because
    /// they are configurable.
    pub fn worst_health(&self) -> Option<Health> {
        let volumes = self.volumes.iter().map(|v| v.health);
        let disks = self.disks.iter().map(|d| d.health);
        let fans = self.fans.iter().map(|f| f.health);
        let alerts = self.alerts.iter().map(|a| match a.level {
            AlertLevel::Info => None,
            AlertLevel::Warning => Some(Health::Warning),
            AlertLevel::Critical => Some(Health::Critical),
        });
        let backups = self.backups.iter().map(|b| match b.last_result {
            Some(BackupResult::Ok) => Some(Health::Ok),
            Some(BackupResult::Error) => Some(Health::Warning),
            Some(BackupResult::Running | BackupResult::Never) | None => None,
        });
        let ups = self.ups.iter().map(|u| match u.state {
            Some(UpsState::Online) => Some(Health::Ok),
            Some(UpsState::OnBattery) => Some(Health::Warning),
            None => None,
        });
        Health::worst(
            volumes
                .chain(disks)
                .chain(fans)
                .chain(alerts)
                .chain(backups)
                .chain(ups),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupKind;
    use crate::nas::ProviderKind;
    use crate::storage::{ScrubState, ScrubStatus, VolumeKind};

    fn empty() -> Snapshot {
        Snapshot {
            nas: Nas {
                id: "nas-1".into(),
                provider: ProviderKind::Qnap,
                name: None,
                base_url: "https://nas.example:8080".into(),
                model: None,
                firmware: None,
                hostname: None,
            },
            observed_at: 1_700_000_000,
            system: SystemStatus::default(),
            ups: None,
            volumes: Vec::new(),
            disks: Vec::new(),
            fans: Vec::new(),
            backups: Vec::new(),
            containers: Vec::new(),
            alerts: Vec::new(),
            interfaces: Vec::new(),
        }
    }

    fn alert(level: AlertLevel) -> Alert {
        Alert {
            level,
            text: "Test".into(),
            since: None,
        }
    }

    fn backup(result: Option<BackupResult>) -> BackupTask {
        BackupTask {
            id: "b1".into(),
            name: "Backup".into(),
            kind: BackupKind::Hbs3,
            last_run: None,
            last_result: result,
            next_run: None,
        }
    }

    fn fan(health: Option<Health>) -> Fan {
        Fan {
            name: "Fan 1".into(),
            rpm: None,
            health,
        }
    }

    fn ups(state: Option<UpsState>) -> Ups {
        Ups {
            state,
            charge_percent: None,
            runtime_seconds: None,
        }
    }

    fn full() -> Snapshot {
        let mut snapshot = empty();
        snapshot.nas.name = Some("Demo NAS".into());
        snapshot.nas.model = Some("TS-000".into());
        snapshot.nas.firmware = Some("5.2.0".into());
        snapshot.nas.hostname = Some("demo-nas".into());
        snapshot.system = SystemStatus {
            uptime_seconds: Some(86_400),
            cpu_percent: Some(12.5),
            memory_percent: Some(40.25),
            temperature_c: Some(41.0),
            update_available: Some(true),
            update_version: Some("5.2.1".into()),
        };
        snapshot.ups = Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: Some(100.0),
            runtime_seconds: Some(1_800),
        });
        snapshot.volumes = vec![
            Volume {
                id: "pool-1".into(),
                name: "Pool 1".into(),
                kind: VolumeKind::Pool,
                pool_id: None,
                used_bytes: Some(1_000),
                total_bytes: Some(4_000),
                health: Some(Health::Ok),
                status_text: Some("Ready".into()),
                raid: Some("RAID 5".into()),
                scrub: Some(ScrubStatus {
                    state: Some(ScrubState::Finished),
                    last_finished_at: Some(1_699_990_000),
                    errors: Some(0),
                    progress_percent: None,
                }),
            },
            Volume {
                id: "vol-1".into(),
                name: "Volume 1".into(),
                kind: VolumeKind::Volume,
                pool_id: Some("pool-1".into()),
                used_bytes: Some(500),
                total_bytes: Some(2_000),
                health: None,
                status_text: None,
                raid: None,
                scrub: None,
            },
        ];
        snapshot.disks = vec![Disk {
            id: "disk-1".into(),
            name: "Disk 1".into(),
            model: Some("Demo HDD".into()),
            serial_hash: Some(crate::hash_serial("nas-1", "DEMO-SERIAL")),
            temperature_c: Some(35.5),
            health: Some(Health::Ok),
            smart_text: Some("Good".into()),
            power_on_hours: Some(12_345),
            spinning: Some(true),
        }];
        snapshot.fans = vec![Fan {
            name: "Fan 1".into(),
            rpm: Some(900),
            health: Some(Health::Ok),
        }];
        snapshot.backups = vec![BackupTask {
            id: "b1".into(),
            name: "Nightly".into(),
            kind: BackupKind::Hbs3,
            last_run: Some(1_699_950_000),
            last_result: Some(BackupResult::Ok),
            next_run: Some(1_700_036_400),
        }];
        snapshot.containers = vec![Container {
            name: "demo-app".into(),
            running: true,
        }];
        snapshot.alerts = vec![Alert {
            level: AlertLevel::Info,
            text: "Demo notice".into(),
            since: Some(1_699_000_000),
        }];
        snapshot.interfaces = vec![NetInterface {
            name: "eth0".into(),
            rx_bytes_per_sec: Some(1_024.0),
            tx_bytes_per_sec: Some(512.0),
        }];
        snapshot
    }

    #[test]
    fn nothing_known_is_none() {
        assert_eq!(empty().worst_health(), None);
    }

    #[test]
    fn only_info_alerts_is_none() {
        let mut snapshot = empty();
        snapshot.alerts = vec![alert(AlertLevel::Info)];
        assert_eq!(snapshot.worst_health(), None);
    }

    #[test]
    fn alerts_map_to_their_level() {
        let mut snapshot = empty();
        snapshot.alerts = vec![alert(AlertLevel::Warning)];
        assert_eq!(snapshot.worst_health(), Some(Health::Warning));
        snapshot.alerts.push(alert(AlertLevel::Critical));
        assert_eq!(snapshot.worst_health(), Some(Health::Critical));
    }

    #[test]
    fn ups_on_battery_is_warning() {
        let mut snapshot = empty();
        snapshot.ups = Some(ups(Some(UpsState::OnBattery)));
        assert_eq!(snapshot.worst_health(), Some(Health::Warning));
        snapshot.ups = Some(ups(Some(UpsState::Online)));
        assert_eq!(snapshot.worst_health(), Some(Health::Ok));
        snapshot.ups = Some(ups(None));
        assert_eq!(snapshot.worst_health(), None);
    }

    #[test]
    fn backup_results_count() {
        let mut snapshot = empty();
        snapshot.backups = vec![
            backup(Some(BackupResult::Running)),
            backup(Some(BackupResult::Never)),
            backup(None),
        ];
        assert_eq!(snapshot.worst_health(), None);
        snapshot.backups.push(backup(Some(BackupResult::Ok)));
        assert_eq!(snapshot.worst_health(), Some(Health::Ok));
        snapshot.backups.push(backup(Some(BackupResult::Error)));
        assert_eq!(snapshot.worst_health(), Some(Health::Warning));
    }

    #[test]
    fn critical_wins_over_every_source() {
        let mut snapshot = full();
        assert_eq!(snapshot.worst_health(), Some(Health::Ok));
        snapshot.fans.push(fan(Some(Health::Critical)));
        snapshot.ups = Some(ups(Some(UpsState::OnBattery)));
        assert_eq!(snapshot.worst_health(), Some(Health::Critical));
    }

    #[test]
    fn volumes_and_disks_count() {
        let mut snapshot = full();
        snapshot.volumes[1].health = Some(Health::Warning);
        assert_eq!(snapshot.worst_health(), Some(Health::Warning));
        snapshot.disks[0].health = Some(Health::Critical);
        assert_eq!(snapshot.worst_health(), Some(Health::Critical));
    }

    #[test]
    fn full_snapshot_survives_json() {
        let snapshot = full();
        let json = serde_json::to_string(&snapshot).expect("snapshot serializes");
        let back: Snapshot = serde_json::from_str(&json).expect("snapshot deserializes");
        assert_eq!(back, snapshot);
    }
}
