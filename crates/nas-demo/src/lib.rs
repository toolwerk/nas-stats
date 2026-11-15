//! Deterministic demo data for NAS Stats.
//!
//! Three fictional NAS as in `docs/mockup/nas-stats-mockup.html` (Keller,
//! Büro, Homelab) with their current state, 90 days of daily history and the
//! runs of their backup tasks. Screenshots, store images and the demo mode use
//! this instead of real NAS data.
//!
//! All times hang on the `now` passed to [`demo`], so the data always looks
//! fresh. The same `now` yields the same values on every platform: the
//! generator is integer based and no trigonometry is used.
//!
//! The data marks itself as demo: every id starts with [`DEMO_ID_PREFIX`],
//! every base URL ends in `.demo.invalid` and every hostname in `-demo`. The
//! visible "Demo" label is up to the apps and widgets.

mod history;
mod rng;
mod snapshots;

use nas_model::{BackupRun, Sample, Snapshot};

pub use rng::DEMO_SEED;

/// Prefix of every id in the demo data.
pub const DEMO_ID_PREFIX: &str = "demo-";

/// Domain every demo base URL ends in. `.invalid` never resolves (RFC 2606).
pub const DEMO_DOMAIN: &str = ".demo.invalid";

/// The whole demo data set.
#[derive(Debug, Clone, PartialEq)]
pub struct DemoSet {
    /// Keller, Büro and Homelab, in this order.
    pub snapshots: Vec<Snapshot>,
    /// 90 daily values per series, oldest first, the last at `now`.
    pub samples: Vec<Sample>,
    /// One run per day for each backup task except snapshots.
    pub backup_runs: Vec<BackupRun>,
}

/// Builds the demo data for the time `now` (Unix seconds UTC).
///
/// Any realistic `now` works (it must stay at least 90 days and one day away
/// from the limits of `i64`); the same `now` always yields the same data.
pub fn demo(now: i64) -> DemoSet {
    let snapshots = snapshots::snapshots(now);
    let samples = history::samples(now, &snapshots);
    let backup_runs = history::backup_runs(&snapshots);
    DemoSet {
        snapshots,
        samples,
        backup_runs,
    }
}

/// Whether `id` belongs to the demo data.
pub fn is_demo_id(id: &str) -> bool {
    id.starts_with(DEMO_ID_PREFIX)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nas_model::{Metric, ProviderKind, Volume};

    use super::*;
    use crate::history::{DATAVOL1_DAYS_TO_FULL, DAYS, TREND_DAYS};

    /// 14 November 2026, 14:05 UTC.
    const NOW: i64 = 1_794_665_100;

    fn snapshot<'a>(set: &'a DemoSet, id: &str) -> &'a Snapshot {
        set.snapshots
            .iter()
            .find(|s| s.nas.id == id)
            .expect("demo NAS exists")
    }

    fn volume<'a>(set: &'a DemoSet, nas_id: &str, name: &str) -> &'a Volume {
        snapshot(set, nas_id)
            .volumes
            .iter()
            .find(|v| v.name == name)
            .expect("demo volume exists")
    }

    fn series(set: &DemoSet, nas_id: &str, metric: Metric, key: Option<&str>) -> Vec<f64> {
        set.samples
            .iter()
            .filter(|s| s.nas_id == nas_id && s.metric == metric && s.key.as_deref() == key)
            .map(|s| s.value)
            .collect()
    }

    fn rounded(value: Option<f64>, decimals: i32) -> f64 {
        let factor = 10f64.powi(decimals);
        (value.expect("value is set") * factor).round() / factor
    }

    /// Slope of the least squares line through `values` over x = 0, 1, 2 ...
    fn slope(values: &[f64]) -> f64 {
        let n = values.len() as f64;
        let mean_x = (n - 1.0) / 2.0;
        let mean_y = values.iter().sum::<f64>() / n;
        let (mut num, mut den) = (0.0, 0.0);
        for (x, y) in values.iter().enumerate() {
            let dx = x as f64 - mean_x;
            num += dx * (y - mean_y);
            den += dx * dx;
        }
        num / den
    }

    fn last_trend_days(values: &[f64]) -> &[f64] {
        &values[values.len() - TREND_DAYS..]
    }

    #[test]
    fn same_now_gives_same_data() {
        assert_eq!(demo(NOW), demo(NOW));
    }

    #[test]
    fn three_nas_in_mockup_order() {
        let set = demo(NOW);
        let ids: Vec<_> = set.snapshots.iter().map(|s| s.nas.id.as_str()).collect();
        assert_eq!(ids, ["demo-keller", "demo-buero", "demo-homelab"]);
        assert!(set.snapshots.iter().all(|s| s.observed_at == NOW));
    }

    #[test]
    fn values_match_the_mockup() {
        let set = demo(NOW);

        let temps = |s: &Snapshot| -> Vec<f64> {
            s.disks
                .iter()
                .map(|d| d.temperature_c.expect("temperature"))
                .collect()
        };
        let keller = snapshot(&set, "demo-keller");
        assert_eq!(keller.nas.provider, ProviderKind::Qnap);
        assert_eq!(keller.nas.model.as_deref(), Some("TS-464"));
        assert_eq!(
            keller.nas.firmware.as_deref(),
            Some("5.2.4.3079 Build 20250321")
        );
        assert_eq!(
            rounded(volume(&set, "demo-keller", "DataVol1").used_percent(), 1),
            87.1
        );
        assert_eq!(temps(keller), [38.0, 41.0, 52.0, 39.0]);
        assert_eq!(keller.disks[2].name, "Platte 3");
        assert_eq!(keller.disks[2].temperature_c, Some(52.0));
        assert_eq!(keller.containers.iter().filter(|c| c.running).count(), 6);
        assert_eq!(keller.containers.len(), 7);
        let backups: Vec<_> = keller.backups.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(backups, ["Offsite B2", "Snapshots DataVol1"]);
        assert_eq!(keller.backups[0].last_run, Some(NOW - 9 * 3600));
        assert_eq!(keller.backups[0].kind, nas_model::BackupKind::Hbs3);
        assert_eq!(keller.backups[1].kind, nas_model::BackupKind::Snapshot);
        assert_eq!(keller.backups[1].last_run, Some(NOW - 5 * 60));

        let buero = snapshot(&set, "demo-buero");
        assert_eq!(buero.nas.provider, ProviderKind::Synology);
        assert_eq!(buero.nas.model.as_deref(), Some("DS923+"));
        assert_eq!(buero.nas.firmware.as_deref(), Some("DSM 7.3.1"));
        assert_eq!(
            rounded(volume(&set, "demo-buero", "Volume 1").used_percent(), 1),
            52.0
        );
        assert!(buero.ups.is_none());
        assert_eq!(buero.backups.len(), 1);
        assert_eq!(buero.backups[0].name, "Cloud C2");
        assert_eq!(buero.backups[0].last_run, Some(NOW - 3 * 3600));
        assert_eq!(buero.backups[0].kind, nas_model::BackupKind::HyperBackup);
        assert_eq!(temps(buero), [36.0, 37.0, 39.0, 38.0]);

        let homelab = snapshot(&set, "demo-homelab");
        assert_eq!(homelab.nas.provider, ProviderKind::TrueNas);
        assert_eq!(homelab.nas.model.as_deref(), Some("TrueNAS CE 25.10"));
        assert_eq!(homelab.nas.firmware.as_deref(), Some("25.10.1"));
        assert_eq!(
            rounded(volume(&set, "demo-homelab", "tank").used_percent(), 1),
            64.0
        );
        assert_eq!(
            rounded(volume(&set, "demo-homelab", "apps").used_percent(), 0),
            23.0
        );
        assert_eq!(homelab.backups.len(), 1);
        assert_eq!(homelab.backups[0].name, "Replikation tank");
        assert_eq!(homelab.backups[0].last_run, Some(NOW - 3600));
        assert_eq!(homelab.backups[0].kind, nas_model::BackupKind::Replication);
        // 4 HDD at 40 to 44 °C, then 2 SSD.
        assert_eq!(temps(homelab), [40.0, 42.0, 44.0, 41.0, 33.0, 34.0]);
        assert!(homelab.disks[4..].iter().all(|d| {
            d.model
                .as_deref()
                .is_some_and(|m| m.contains("SSD") || m.contains("EVO"))
                && d.spinning.is_none()
        }));
        assert!(homelab.disks[..4].iter().all(|d| d.spinning == Some(true)));
    }

    #[test]
    fn keller_crosses_the_warning_thresholds() {
        let set = demo(NOW);
        let keller = snapshot(&set, "demo-keller");
        assert!(keller.disks[2].temperature_c.expect("temperature") > 50.0);
        let datavol1 = volume(&set, "demo-keller", "DataVol1");
        assert!(datavol1.used_percent().expect("percent") > 80.0);
    }

    #[test]
    fn everything_is_marked_as_demo() {
        let set = demo(NOW);
        for s in &set.snapshots {
            assert!(is_demo_id(&s.nas.id), "{}", s.nas.id);
            assert!(s.nas.base_url.ends_with(DEMO_DOMAIN), "{}", s.nas.base_url);
            let hostname = s.nas.hostname.as_deref().expect("hostname");
            assert!(hostname.ends_with("-demo"), "{hostname}");
            for v in &s.volumes {
                assert!(is_demo_id(&v.id), "{}", v.id);
                if let Some(pool) = &v.pool_id {
                    assert!(is_demo_id(pool), "{pool}");
                }
            }
            for d in &s.disks {
                assert!(is_demo_id(&d.id), "{}", d.id);
            }
            for b in &s.backups {
                assert!(is_demo_id(&b.id), "{}", b.id);
            }
        }
        for sample in &set.samples {
            assert!(is_demo_id(&sample.nas_id));
            if matches!(
                sample.metric,
                Metric::VolumeUsed | Metric::VolumeTotal | Metric::DiskTemp
            ) {
                let key = sample.key.as_deref().expect("key");
                assert!(is_demo_id(key), "{key}");
            }
        }
        assert!(set.backup_runs.iter().all(|r| is_demo_id(&r.task_id)));
        assert!(!is_demo_id("keller"));
    }

    #[test]
    fn every_series_has_90_days_ending_with_the_snapshot() {
        let set = demo(NOW);
        let mut groups: BTreeMap<(String, &str, Option<String>), Vec<&Sample>> = BTreeMap::new();
        for s in &set.samples {
            assert!(s.at <= NOW, "{s:?}");
            groups
                .entry((s.nas_id.clone(), s.metric.as_str(), s.key.clone()))
                .or_default()
                .push(s);
        }
        for ((nas_id, _, key), group) in &groups {
            assert_eq!(group.len(), DAYS, "{nas_id} {key:?}");
            assert!(group.windows(2).all(|w| w[1].at - w[0].at == 86_400));
            let last = group[DAYS - 1];
            assert_eq!(last.at, NOW);
            let snap = snapshot(&set, nas_id);
            let key = key.as_deref();
            let expected = match last.metric {
                Metric::VolumeUsed | Metric::VolumeTotal => {
                    let v = snap.volumes.iter().find(|v| Some(v.id.as_str()) == key);
                    let v = v.expect("volume of the series");
                    let bytes = if last.metric == Metric::VolumeUsed {
                        v.used_bytes
                    } else {
                        v.total_bytes
                    };
                    bytes.map(|b| b as f64)
                }
                Metric::DiskTemp => snap
                    .disks
                    .iter()
                    .find(|d| Some(d.id.as_str()) == key)
                    .and_then(|d| d.temperature_c),
                Metric::Cpu => snap.system.cpu_percent,
                Metric::Mem => snap.system.memory_percent,
                Metric::SysTemp => snap.system.temperature_c,
                Metric::NetRx | Metric::NetTx => {
                    let i = snap
                        .interfaces
                        .iter()
                        .find(|i| Some(i.name.as_str()) == key);
                    let i = i.expect("interface of the series");
                    if last.metric == Metric::NetRx {
                        i.rx_bytes_per_sec
                    } else {
                        i.tx_bytes_per_sec
                    }
                }
            };
            assert_eq!(Some(last.value), expected, "{nas_id} {key:?}");
        }
        // Volumes with bytes (pool 1 of Keller has none), disks, system and net.
        let disks: usize = set.snapshots.iter().map(|s| s.disks.len()).sum();
        let volumes = 4;
        assert_eq!(groups.len(), volumes * 2 + disks + 3 * 3 + 3 * 2);
    }

    #[test]
    fn datavol1_is_full_in_143_days() {
        let set = demo(NOW);
        let datavol1 = volume(&set, "demo-keller", "DataVol1");
        let used = series(&set, "demo-keller", Metric::VolumeUsed, Some(&datavol1.id));
        let growth = slope(last_trend_days(&used));
        let total = datavol1.total_bytes.expect("total") as f64;
        let last = used[DAYS - 1];
        assert_eq!(((total - last) / growth).round(), DATAVOL1_DAYS_TO_FULL);
        // The mockup shows +0.09 % per day.
        assert_eq!((growth / total * 10_000.0).round(), 9.0);
    }

    #[test]
    fn apps_has_no_growing_trend() {
        let set = demo(NOW);
        let apps = volume(&set, "demo-homelab", "apps");
        let used = series(&set, "demo-homelab", Metric::VolumeUsed, Some(&apps.id));
        assert!(slope(last_trend_days(&used)) <= 0.0);
    }

    #[test]
    fn datavol1_was_cleaned_up_on_day_41() {
        let set = demo(NOW);
        let datavol1 = volume(&set, "demo-keller", "DataVol1");
        let used = series(&set, "demo-keller", Metric::VolumeUsed, Some(&datavol1.id));
        assert!(used[41] < used[40]);
        let total = datavol1.total_bytes.expect("total") as f64;
        assert!(used.iter().all(|&u| (0.0..=total).contains(&u)));
    }

    #[test]
    fn platte_3_peaks_today() {
        let set = demo(NOW);
        let temps = series(
            &set,
            "demo-keller",
            Metric::DiskTemp,
            Some("demo-keller-disk3"),
        );
        assert_eq!(temps[DAYS - 1], 52.0);
        assert!(temps[..DAYS - 1].iter().all(|&t| t < 52.0));
    }

    #[test]
    fn backup_runs_end_with_the_last_run() {
        let set = demo(NOW);
        for s in &set.snapshots {
            for task in &s.backups {
                let runs: Vec<_> = set
                    .backup_runs
                    .iter()
                    .filter(|r| r.task_id == task.id)
                    .collect();
                if task.kind == nas_model::BackupKind::Snapshot {
                    assert!(runs.is_empty());
                    continue;
                }
                assert_eq!(runs.len(), DAYS);
                assert_eq!(runs[DAYS - 1].started_at, task.last_run.expect("last run"));
                for run in &runs {
                    let finished = run.finished_at.expect("finished");
                    assert_eq!(finished - run.started_at, 1800);
                    assert!(finished <= NOW);
                    assert_eq!(run.result, nas_model::BackupResult::Ok);
                }
            }
        }
    }

    #[test]
    fn health_stays_ok_without_thresholds() {
        let set = demo(NOW);
        for s in &set.snapshots {
            assert_eq!(
                s.worst_health(),
                Some(nas_model::Health::Ok),
                "{}",
                s.nas.id
            );
            assert!(s.alerts.is_empty());
        }
    }

    #[test]
    fn other_now_moves_the_times_only() {
        let a = demo(NOW);
        let b = demo(NOW + 3600);
        let values = |set: &DemoSet| set.samples.iter().map(|s| s.value).collect::<Vec<_>>();
        assert_eq!(values(&a), values(&b));
        assert_eq!(a.samples[0].at + 3600, b.samples[0].at);
    }
}
