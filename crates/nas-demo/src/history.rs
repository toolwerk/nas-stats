//! 90 days of history and backup runs, derived from the current state.
//!
//! Every series ends with the value of the snapshot. The used space of each
//! volume is built so that a linear regression over the last 30 days yields
//! exactly the chosen growth per day: around a straight line, the noise is
//! mirrored at the middle of the 30 days and is zero at both ends.

use nas_model::{BackupKind, BackupResult, BackupRun, Metric, Sample, Snapshot};

use crate::rng::{DemoRng, Seeds};

/// Number of daily values per series.
pub(crate) const DAYS: usize = 90;
/// Days at the end of each volume series whose regression yields the growth.
pub(crate) const TREND_DAYS: usize = 30;

const DAY: i64 = 86_400;
const LAST: usize = DAYS - 1;
const TREND_START: usize = DAYS - TREND_DAYS;
/// Noise of the used space per day, as share of the total space.
const VOLUME_NOISE: f64 = 0.0012;
/// Run time of each backup run, in seconds.
const BACKUP_DURATION: i64 = 30 * 60;

/// Weekly pattern in `-1..1`, a sine sampled by hand (no trigonometry, so
/// every platform yields the same values).
const WEEK: [f64; 7] = [0.0, 0.78, 0.97, 0.43, -0.43, -0.97, -0.78];

/// Id of DataVol1 of Keller.
const DATAVOL1_ID: &str = "demo-keller-datavol1";
/// Days until full for DataVol1, as in the mockup.
pub(crate) const DATAVOL1_DAYS_TO_FULL: f64 = 143.0;
const VOLUME1_DAYS_TO_FULL: f64 = 657.0;
const TANK_DAYS_TO_FULL: f64 = 876.0;
/// apps shrinks a little, so it shows no growing trend.
const APPS_GROWTH_PER_DAY: f64 = -200_000_000.0;

/// Time of day `d` (0 is the oldest, 89 is `now`).
fn day_at(now: i64, d: usize) -> i64 {
    now - (LAST - d) as i64 * DAY
}

fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// All samples of all snapshots, series by series in output order.
pub(crate) fn samples(now: i64, snapshots: &[Snapshot]) -> Vec<Sample> {
    let mut seeds = Seeds::default();
    let mut out = Vec::new();
    for snapshot in snapshots {
        let nas_id = snapshot.nas.id.as_str();
        let mut push = |metric: Metric, key: Option<&str>, values: Vec<f64>| {
            out.extend(values.into_iter().enumerate().map(|(d, value)| Sample {
                nas_id: nas_id.to_owned(),
                metric,
                key: key.map(str::to_owned),
                at: day_at(now, d),
                value,
            }));
        };

        for volume in &snapshot.volumes {
            let (Some(used), Some(total)) = (volume.used_bytes, volume.total_bytes) else {
                continue;
            };
            let key = Some(volume.id.as_str());
            let growth = growth_per_day(&volume.id, used, total).unwrap_or(0.0);
            let cleanup = volume.id == DATAVOL1_ID;
            push(
                Metric::VolumeUsed,
                key,
                volume_used(&mut seeds.series(), used, total, growth, cleanup),
            );
            // Constant, but it still takes its place in the seed order.
            let _ = seeds.series();
            push(Metric::VolumeTotal, key, vec![total as f64; DAYS]);
        }

        for disk in &snapshot.disks {
            let Some(end) = disk.temperature_c else {
                continue;
            };
            // Platte 3 of Keller runs a bit cooler before today's peak.
            let offset = if disk.id == "demo-keller-disk3" {
                -2.5
            } else {
                0.0
            };
            let values = daily(&mut seeds.series(), end, 1.0, 1.2, offset, 0.0);
            push(Metric::DiskTemp, Some(disk.id.as_str()), values);
        }

        let system = &snapshot.system;
        if let Some(end) = system.cpu_percent {
            let values = daily(&mut seeds.series(), end, 3.0, 8.0, 0.0, 2.0);
            push(Metric::Cpu, None, values);
        }
        if let Some(end) = system.memory_percent {
            let values = daily(&mut seeds.series(), end, 2.0, 3.0, 0.0, 0.0);
            push(Metric::Mem, None, values);
        }
        if let Some(end) = system.temperature_c {
            let values = daily(&mut seeds.series(), end, 1.0, 1.0, 0.0, 0.0);
            push(Metric::SysTemp, None, values);
        }

        for interface in &snapshot.interfaces {
            let key = Some(interface.name.as_str());
            if let Some(end) = interface.rx_bytes_per_sec {
                let values = daily(&mut seeds.series(), end, 0.3 * end, 0.4 * end, 0.0, 0.0);
                push(Metric::NetRx, key, values);
            }
            if let Some(end) = interface.tx_bytes_per_sec {
                let values = daily(&mut seeds.series(), end, 0.3 * end, 0.4 * end, 0.0, 0.0);
                push(Metric::NetTx, key, values);
            }
        }
    }
    out
}

/// Growth of a volume in bytes per day, by volume id.
///
/// `None` for an id without a set growth; a test makes sure every demo
/// volume with bytes has one, so a renamed volume cannot go flat unnoticed.
pub(crate) fn growth_per_day(volume_id: &str, used: u64, total: u64) -> Option<f64> {
    let free = total.saturating_sub(used) as f64;
    match volume_id {
        DATAVOL1_ID => Some(free / DATAVOL1_DAYS_TO_FULL),
        "demo-buero-volume1" => Some(free / VOLUME1_DAYS_TO_FULL),
        "demo-homelab-tank" => Some(free / TANK_DAYS_TO_FULL),
        "demo-homelab-apps" => Some(APPS_GROWTH_PER_DAY),
        _ => None,
    }
}

/// Used bytes per day, ending with `used`.
///
/// Days 60 to 89 lie on a line with slope `growth` plus mirrored noise, so
/// the regression over them yields `growth`. Days 0 to 59 run backwards from
/// day 60 with `growth` plus noise. With `cleanup` (DataVol1) the older part
/// follows the mockup instead: 0.07 % of the total per day up to day 40,
/// 0.09 % from day 41, and day 41 lies 2.6 points below the trend.
fn volume_used(rng: &mut DemoRng, used: u64, total: u64, growth: f64, cleanup: bool) -> Vec<f64> {
    let used = used as f64;
    let total = total as f64;
    let noise_amplitude = VOLUME_NOISE * total;
    let mut values = vec![0.0; DAYS];

    // Mirrored noise: n(60 + k) == n(89 - k), zero at both ends.
    let mut noise = [0.0; TREND_DAYS];
    for k in 1..TREND_DAYS / 2 {
        let n = rng.centered() * noise_amplitude;
        noise[k] = n;
        noise[TREND_DAYS - 1 - k] = n;
    }
    for d in TREND_START..DAYS {
        values[d] = used - growth * (LAST - d) as f64 + noise[d - TREND_START];
    }

    // Older days, backwards from day 60: values[d - 1] = values[d] - step(d).
    for d in (1..=TREND_START).rev() {
        let step = if cleanup {
            let base = if d <= 40 { 0.0007 } else { 0.0009 };
            let drop = if d == 41 { 0.026 } else { 0.0 };
            (base - drop) * total
        } else {
            growth
        };
        values[d - 1] = values[d] - step - rng.centered() * noise_amplitude;
    }

    values
        .into_iter()
        .map(|v| v.round().clamp(0.0, total))
        .collect()
}

/// Daily values around `end`: weekly pattern times `amplitude`, noise times
/// `noise`, plus `offset`, at least `min`, rounded to a tenth. The last day
/// is exactly `end`.
fn daily(
    rng: &mut DemoRng,
    end: f64,
    amplitude: f64,
    noise: f64,
    offset: f64,
    min: f64,
) -> Vec<f64> {
    (0..DAYS)
        .map(|d| {
            let value = end + WEEK[d % WEEK.len()] * amplitude + rng.centered() * noise + offset;
            if d == LAST {
                end
            } else {
                round_tenth(value.max(min))
            }
        })
        .collect()
}

/// One run per day for every backup task except snapshots, all successful.
/// The last run starts at the task's `last_run`.
pub(crate) fn backup_runs(snapshots: &[Snapshot]) -> Vec<BackupRun> {
    snapshots
        .iter()
        .flat_map(|snapshot| &snapshot.backups)
        .filter(|task| task.kind != BackupKind::Snapshot)
        .filter_map(|task| task.last_run.map(|last| (task, last)))
        .flat_map(|(task, last)| {
            (0..DAYS).map(move |d| {
                let started_at = last - (LAST - d) as i64 * DAY;
                BackupRun {
                    task_id: task.id.clone(),
                    started_at,
                    finished_at: Some(started_at + BACKUP_DURATION),
                    result: BackupResult::Ok,
                    message: None,
                }
            })
        })
        .collect()
}
