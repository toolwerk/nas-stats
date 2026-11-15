//! Findings: what about a NAS needs attention, with level and text.
//!
//! The NAS reports the health of its parts itself (see
//! [`Snapshot::worst_health`]); thresholds such as "disk too hot" or "volume
//! almost full" are checked here, so the apps, events and notifications of
//! every platform share one list.

use serde::{Deserialize, Serialize};

use crate::backup::{BackupKind, BackupResult};
use crate::device::{AlertLevel, UpsState};
use crate::health::Health;
use crate::snapshot::Snapshot;
use crate::storage::{Volume, VolumeKind};

/// Limits from which a value becomes a [`Finding`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Disk temperature in degrees Celsius from which a disk is a warning.
    pub disk_temp_warning_c: f64,
    /// Fill level in percent from which a volume is a warning.
    pub fill_warning_percent: f64,
    /// Fill level in percent from which a volume is critical.
    pub fill_critical_percent: f64,
    /// Age of the last backup run in seconds above which it is a warning.
    pub backup_max_age_secs: i64,
}

impl Default for Thresholds {
    /// 50 °C, 80 and 90 percent, two days.
    fn default() -> Self {
        Self {
            disk_temp_warning_c: 50.0,
            fill_warning_percent: 80.0,
            fill_critical_percent: 90.0,
            backup_max_age_secs: 2 * 86_400,
        }
    }
}

/// The rule a [`Finding`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// An alert of the NAS with level warning or critical.
    Alert,
    /// A disk whose SMART state the NAS reports as not good.
    DiskHealth,
    /// A disk at or above the temperature threshold.
    DiskTemperature,
    /// A volume, pool, array or share the NAS reports as not good.
    VolumeHealth,
    /// A volume at or above a fill threshold.
    VolumeFill,
    /// A fan the NAS reports as not good.
    FanHealth,
    /// The UPS runs on battery.
    UpsOnBattery,
    /// The last run of a backup task failed.
    BackupFailed,
    /// The last run of a backup task is older than allowed.
    BackupOverdue,
}

/// Something about a NAS that needs attention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    /// `Warning` or `Critical`, never `Ok`.
    pub level: Health,
    /// Id of the disk, volume or backup task; the name for a fan; `None`
    /// for alerts and the UPS.
    pub subject: Option<String>,
    /// German text for the apps, e.g. „Platte 3 bei 52 °C“.
    pub text: String,
}

impl Snapshot {
    /// Volumes whose fill level counts: those of the first kind present in
    /// the order volume, pool, array, share. A QNAP pool and the volume in
    /// it would otherwise count twice.
    pub fn capacity_volumes(&self) -> Vec<&Volume> {
        [
            VolumeKind::Volume,
            VolumeKind::Pool,
            VolumeKind::Array,
            VolumeKind::Share,
        ]
        .into_iter()
        .map(|kind| {
            self.volumes
                .iter()
                .filter(|v| v.kind == kind)
                .collect::<Vec<_>>()
        })
        .find(|volumes| !volumes.is_empty())
        .unwrap_or_default()
    }

    /// Everything that needs attention, critical before warning; within a
    /// level alerts, disks, volumes, fans, UPS, backups. A missing value
    /// yields no finding. Numbers in the texts are rounded down, the
    /// thresholds compare the raw values.
    pub fn findings(&self, thresholds: &Thresholds) -> Vec<Finding> {
        let mut findings = Vec::new();

        for alert in &self.alerts {
            let level = match alert.level {
                AlertLevel::Info => continue,
                AlertLevel::Warning => Health::Warning,
                AlertLevel::Critical => Health::Critical,
            };
            findings.push(Finding {
                kind: FindingKind::Alert,
                level,
                subject: None,
                text: alert.text.clone(),
            });
        }

        for disk in &self.disks {
            if let Some(level) = not_ok(disk.health) {
                let text = match &disk.smart_text {
                    Some(smart) => format!("{}: SMART {smart}", disk.name),
                    None => format!("{}: SMART nicht in Ordnung", disk.name),
                };
                findings.push(Finding {
                    kind: FindingKind::DiskHealth,
                    level,
                    subject: Some(disk.id.clone()),
                    text,
                });
            }
            if let Some(temp) = disk.temperature_c
                && temp >= thresholds.disk_temp_warning_c
            {
                findings.push(Finding {
                    kind: FindingKind::DiskTemperature,
                    level: Health::Warning,
                    subject: Some(disk.id.clone()),
                    text: format!("{} bei {} °C", disk.name, temp.floor()),
                });
            }
        }

        for volume in &self.volumes {
            if let Some(level) = not_ok(volume.health) {
                let text = match &volume.status_text {
                    Some(status) => format!("{}: {status}", volume.name),
                    None => format!("{}: Zustand gestört", volume.name),
                };
                findings.push(Finding {
                    kind: FindingKind::VolumeHealth,
                    level,
                    subject: Some(volume.id.clone()),
                    text,
                });
            }
        }
        for volume in self.capacity_volumes() {
            let Some(percent) = volume.used_percent() else {
                continue;
            };
            let level = if percent >= thresholds.fill_critical_percent {
                Health::Critical
            } else if percent >= thresholds.fill_warning_percent {
                Health::Warning
            } else {
                continue;
            };
            findings.push(Finding {
                kind: FindingKind::VolumeFill,
                level,
                subject: Some(volume.id.clone()),
                text: format!("{} bei {} %", volume.name, percent.floor()),
            });
        }

        for fan in &self.fans {
            if let Some(level) = not_ok(fan.health) {
                findings.push(Finding {
                    kind: FindingKind::FanHealth,
                    level,
                    subject: Some(fan.name.clone()),
                    text: format!("{} gestört", fan.name),
                });
            }
        }

        if let Some(ups) = &self.ups
            && ups.state == Some(UpsState::OnBattery)
        {
            findings.push(Finding {
                kind: FindingKind::UpsOnBattery,
                level: Health::Critical,
                subject: None,
                text: "USV auf Batterie".into(),
            });
        }

        for backup in &self.backups {
            if backup.last_result == Some(BackupResult::Error) {
                findings.push(Finding {
                    kind: FindingKind::BackupFailed,
                    level: Health::Warning,
                    subject: Some(backup.id.clone()),
                    text: format!("Backup „{}“ fehlgeschlagen", backup.name),
                });
                continue;
            }
            // Snapshot tasks and the parity check run on their own schedule
            // (weekly, monthly); their age says nothing about a backup.
            if matches!(
                backup.last_result,
                Some(BackupResult::Running | BackupResult::Never)
            ) || matches!(backup.kind, BackupKind::Snapshot | BackupKind::Parity)
            {
                continue;
            }
            if let Some(last_run) = backup.last_run {
                let age = self.observed_at.saturating_sub(last_run);
                if age > thresholds.backup_max_age_secs {
                    findings.push(Finding {
                        kind: FindingKind::BackupOverdue,
                        level: Health::Warning,
                        subject: Some(backup.id.clone()),
                        text: format!(
                            "Backup „{}“ seit {} Tagen nicht gelaufen",
                            backup.name,
                            age / 86_400
                        ),
                    });
                }
            }
        }

        // Stable: the order of the rules holds within a level.
        findings.sort_by_key(|f| std::cmp::Reverse(f.level));
        findings
    }
}

/// The level of a part the NAS reports as not good.
fn not_ok(health: Option<Health>) -> Option<Health> {
    health.filter(|h| *h != Health::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupTask;
    use crate::device::{Alert, Fan, Ups};
    use crate::nas::{Nas, ProviderKind, SystemStatus};
    use crate::storage::Disk;

    const NOW: i64 = 1_700_000_000;
    const DAY: i64 = 86_400;

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
            observed_at: NOW,
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

    fn volume(id: &str, kind: VolumeKind, used: Option<u64>, total: Option<u64>) -> Volume {
        Volume {
            id: id.into(),
            name: id.into(),
            kind,
            pool_id: None,
            used_bytes: used,
            total_bytes: total,
            health: Some(Health::Ok),
            status_text: None,
            raid: None,
            scrub: None,
        }
    }

    fn disk(n: u32, temperature_c: Option<f64>, health: Option<Health>) -> Disk {
        Disk {
            id: format!("disk-{n}"),
            name: format!("Platte {n}"),
            model: None,
            serial_hash: None,
            temperature_c,
            health,
            smart_text: None,
            power_on_hours: None,
            spinning: None,
        }
    }

    fn backup(last_run: Option<i64>, result: Option<BackupResult>) -> BackupTask {
        BackupTask {
            id: "b1".into(),
            name: "Offsite".into(),
            kind: BackupKind::Hbs3,
            last_run,
            last_result: result,
            next_run: None,
        }
    }

    fn texts(snapshot: &Snapshot) -> Vec<String> {
        snapshot
            .findings(&Thresholds::default())
            .into_iter()
            .map(|f| f.text)
            .collect()
    }

    fn levels(snapshot: &Snapshot) -> Vec<Health> {
        snapshot
            .findings(&Thresholds::default())
            .into_iter()
            .map(|f| f.level)
            .collect()
    }

    #[test]
    fn empty_snapshot_has_no_findings() {
        assert!(empty().findings(&Thresholds::default()).is_empty());
    }

    #[test]
    fn info_alerts_are_skipped() {
        let mut s = empty();
        s.alerts = vec![
            Alert {
                level: AlertLevel::Info,
                text: "Hinweis".into(),
                since: None,
            },
            Alert {
                level: AlertLevel::Critical,
                text: "Lüfter steht".into(),
                since: None,
            },
        ];
        let findings = s.findings(&Thresholds::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::Alert);
        assert_eq!(findings[0].level, Health::Critical);
        assert_eq!(findings[0].text, "Lüfter steht");
        assert_eq!(findings[0].subject, None);
    }

    #[test]
    fn disk_temperature_from_the_threshold() {
        let mut s = empty();
        s.disks = vec![
            disk(1, Some(49.9), None),
            disk(2, Some(50.0), None),
            disk(3, Some(52.7), None),
            disk(4, None, None),
        ];
        assert_eq!(texts(&s), ["Platte 2 bei 50 °C", "Platte 3 bei 52 °C"]);
        let findings = s.findings(&Thresholds::default());
        assert!(
            findings
                .iter()
                .all(|f| f.kind == FindingKind::DiskTemperature)
        );
        assert_eq!(findings[1].subject.as_deref(), Some("disk-3"));
    }

    #[test]
    fn disk_health_with_and_without_smart_text() {
        let mut s = empty();
        s.disks = vec![
            disk(1, None, Some(Health::Ok)),
            disk(2, None, Some(Health::Warning)),
            disk(3, None, Some(Health::Critical)),
        ];
        s.disks[1].smart_text = Some("Warnung".into());
        assert_eq!(
            texts(&s),
            [
                "Platte 3: SMART nicht in Ordnung",
                "Platte 2: SMART Warnung"
            ]
        );
        assert_eq!(levels(&s), [Health::Critical, Health::Warning]);
    }

    #[test]
    fn volume_health_with_and_without_status_text() {
        let mut s = empty();
        s.volumes = vec![
            volume("pool", VolumeKind::Pool, None, None),
            volume("vol", VolumeKind::Volume, None, None),
        ];
        s.volumes[0].health = Some(Health::Critical);
        s.volumes[1].health = Some(Health::Warning);
        s.volumes[1].status_text = Some("Herabgestuft".into());
        assert_eq!(texts(&s), ["pool: Zustand gestört", "vol: Herabgestuft"]);
    }

    #[test]
    fn fill_levels_at_the_limits() {
        let mut s = empty();
        s.volumes = vec![
            volume("a", VolumeKind::Volume, Some(799), Some(1000)),
            volume("b", VolumeKind::Volume, Some(800), Some(1000)),
            volume("c", VolumeKind::Volume, Some(899), Some(1000)),
            volume("d", VolumeKind::Volume, Some(900), Some(1000)),
            volume("e", VolumeKind::Volume, None, Some(1000)),
        ];
        assert_eq!(texts(&s), ["d bei 90 %", "b bei 80 %", "c bei 89 %"]);
        assert_eq!(
            levels(&s),
            [Health::Critical, Health::Warning, Health::Warning]
        );
    }

    #[test]
    fn fill_text_rounds_down() {
        let mut s = empty();
        s.volumes = vec![volume(
            "DataVol1",
            VolumeKind::Volume,
            Some(8_999),
            Some(10_000),
        )];
        let findings = s.findings(&Thresholds::default());
        assert_eq!(findings[0].text, "DataVol1 bei 89 %");
        assert_eq!(findings[0].level, Health::Warning);
    }

    #[test]
    fn only_the_first_kind_counts_for_the_fill() {
        let mut s = empty();
        s.volumes = vec![
            volume("pool", VolumeKind::Pool, Some(95), Some(100)),
            volume("vol", VolumeKind::Volume, Some(50), Some(100)),
        ];
        let ids: Vec<&str> = s.capacity_volumes().iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["vol"]);
        assert!(texts(&s).is_empty());

        s.volumes = vec![
            volume("share", VolumeKind::Share, Some(95), Some(100)),
            volume("array", VolumeKind::Array, Some(85), Some(100)),
        ];
        assert_eq!(texts(&s), ["array bei 85 %"]);
        assert!(empty().capacity_volumes().is_empty());
    }

    #[test]
    fn fan_and_ups() {
        let mut s = empty();
        s.fans = vec![
            Fan {
                name: "Lüfter 1".into(),
                rpm: None,
                health: Some(Health::Warning),
            },
            Fan {
                name: "Lüfter 2".into(),
                rpm: None,
                health: None,
            },
        ];
        s.ups = Some(Ups {
            state: Some(UpsState::OnBattery),
            charge_percent: None,
            runtime_seconds: None,
        });
        assert_eq!(texts(&s), ["USV auf Batterie", "Lüfter 1 gestört"]);
        assert_eq!(levels(&s), [Health::Critical, Health::Warning]);

        s.ups = Some(Ups {
            state: Some(UpsState::Online),
            charge_percent: None,
            runtime_seconds: None,
        });
        assert_eq!(texts(&s), ["Lüfter 1 gestört"]);
    }

    #[test]
    fn failed_backup() {
        let mut s = empty();
        s.backups = vec![backup(Some(NOW - 10 * DAY), Some(BackupResult::Error))];
        assert_eq!(texts(&s), ["Backup „Offsite“ fehlgeschlagen"]);
    }

    #[test]
    fn overdue_backup() {
        let mut s = empty();
        s.backups = vec![backup(Some(NOW - 2 * DAY), Some(BackupResult::Ok))];
        assert!(texts(&s).is_empty());
        s.backups = vec![backup(Some(NOW - 2 * DAY - 1), Some(BackupResult::Ok))];
        assert_eq!(texts(&s), ["Backup „Offsite“ seit 2 Tagen nicht gelaufen"]);
        s.backups = vec![backup(Some(NOW - 5 * DAY), None)];
        assert_eq!(texts(&s), ["Backup „Offsite“ seit 5 Tagen nicht gelaufen"]);
    }

    #[test]
    fn snapshot_and_parity_are_never_overdue() {
        let mut s = empty();
        let mut parity = backup(Some(NOW - 10 * DAY), Some(BackupResult::Ok));
        parity.kind = BackupKind::Parity;
        let mut snapshots = backup(Some(NOW - 7 * DAY), Some(BackupResult::Ok));
        snapshots.kind = BackupKind::Snapshot;
        s.backups = vec![parity, snapshots];
        assert!(texts(&s).is_empty());
        s.backups[0].last_result = Some(BackupResult::Error);
        assert_eq!(texts(&s), ["Backup „Offsite“ fehlgeschlagen"]);
    }

    #[test]
    fn running_or_unknown_backup_is_not_overdue() {
        let mut s = empty();
        s.backups = vec![
            backup(Some(NOW - 5 * DAY), Some(BackupResult::Running)),
            backup(None, Some(BackupResult::Never)),
            backup(Some(NOW - 5 * DAY), Some(BackupResult::Never)),
            backup(None, None),
        ];
        assert!(texts(&s).is_empty());
    }

    #[test]
    fn critical_first_then_the_order_of_the_rules() {
        let mut s = empty();
        s.backups = vec![backup(Some(NOW), Some(BackupResult::Error))];
        s.disks = vec![disk(1, Some(55.0), None)];
        s.volumes = vec![volume("vol", VolumeKind::Volume, Some(95), Some(100))];
        s.alerts = vec![Alert {
            level: AlertLevel::Warning,
            text: "Update verfügbar".into(),
            since: None,
        }];
        assert_eq!(
            texts(&s),
            [
                "vol bei 95 %",
                "Update verfügbar",
                "Platte 1 bei 55 °C",
                "Backup „Offsite“ fehlgeschlagen",
            ]
        );
    }

    #[test]
    fn own_thresholds_apply() {
        let mut s = empty();
        s.disks = vec![disk(1, Some(45.0), None)];
        let thresholds = Thresholds {
            disk_temp_warning_c: 45.0,
            ..Thresholds::default()
        };
        assert_eq!(s.findings(&thresholds).len(), 1);
        assert!(s.findings(&Thresholds::default()).is_empty());
    }
}
