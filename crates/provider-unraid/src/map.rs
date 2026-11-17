//! From the GraphQL answers to the types of `nas-model`.

use nas_model::{
    Alert, AlertLevel, BackupKind, BackupResult, BackupRun, BackupTask, Container, Disk, Health,
    ScrubState, ScrubStatus, SystemStatus, Ups, UpsState, Volume, VolumeKind, hash_serial,
};

use crate::wire::{
    Array, ArrayDisk, DockerContainer, Info, Kilobytes, Metrics, Notification, ParityCheck, Share,
    UpsDevice, local_id, unix_seconds, upper,
};

/// Id of the backup task that stands for the parity check.
pub const PARITY_TASK_ID: &str = "parity";
/// Id of the array volume.
pub(crate) const ARRAY_ID: &str = "array";
/// Prefix of the id of a share volume.
pub(crate) const SHARE_PREFIX: &str = "share:";

/// Unraid counts space in kilobytes of 1000 bytes.
const KB: u64 = 1000;

fn bytes(kilobytes: Option<u64>) -> Option<u64> {
    kilobytes.and_then(|kb| kb.checked_mul(KB))
}

/// Model as manufacturer and model, `None` when both are missing.
pub(crate) fn model(info: Option<&Info>) -> Option<String> {
    let system = info?.system.as_ref()?;
    let parts: Vec<&str> = [&system.manufacturer, &system.model]
        .into_iter()
        .filter_map(|p| p.as_deref())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Unraid version, e.g. `7.2.0`.
pub(crate) fn firmware(info: Option<&Info>) -> Option<String> {
    info?.versions.as_ref()?.core.as_ref()?.unraid.clone()
}

pub(crate) fn hostname(info: Option<&Info>) -> Option<String> {
    info?.os.as_ref()?.hostname.clone()
}

/// Seconds since the boot time Unraid reports in `os.uptime`; `None` for a
/// boot time in the future.
pub(crate) fn uptime(info: Option<&Info>, now: i64) -> Option<u64> {
    let boot = unix_seconds(info?.os.as_ref()?.uptime.as_deref()?)?;
    u64::try_from(now.checked_sub(boot)?).ok()
}

/// CPU and memory; temperatures and the update stay `None`, the API of
/// Unraid 7.2.0 does not report them.
pub(crate) fn system(info: Option<&Info>, metrics: Option<&Metrics>, now: i64) -> SystemStatus {
    let cpu = metrics.and_then(|m| m.cpu.as_ref());
    let memory = metrics.and_then(|m| m.memory.as_ref());
    SystemStatus {
        uptime_seconds: uptime(info, now),
        cpu_percent: cpu.and_then(|c| c.percent_total),
        memory_percent: memory.and_then(|m| m.percent_total),
        memory_total_bytes: memory.and_then(|m| m.total),
        ..SystemStatus::default()
    }
}

/// The array as a volume. Space only while it is started: a stopped array
/// reports no file systems.
pub(crate) fn array_volume(array: &Array) -> Volume {
    let state = upper(array.state.as_deref());
    let started = state.as_deref() == Some("STARTED");
    let kilobytes = array
        .capacity
        .as_ref()
        .and_then(|c| c.kilobytes.as_ref())
        .filter(|_| started);
    let health = match state.as_deref() {
        None => None,
        Some("STARTED") => Some(Health::Ok),
        Some("STOPPED") => Some(Health::Warning),
        Some(_) => Some(Health::Critical),
    };
    Volume {
        id: ARRAY_ID.into(),
        name: "Array".into(),
        kind: VolumeKind::Array,
        pool_id: None,
        used_bytes: bytes(kilobytes.and_then(|k: &Kilobytes| k.used)),
        total_bytes: bytes(kilobytes.and_then(|k| k.total)),
        health,
        status_text: array.state.clone(),
        raid: None,
        scrub: array.parity_check_status.as_ref().and_then(scrub),
    }
}

/// The current parity check as scrub state. `None` when it never ran.
fn scrub(check: &ParityCheck) -> Option<ScrubStatus> {
    let status = upper(check.status.as_deref());
    let state = match status.as_deref() {
        Some("NEVER_RUN") => return None,
        Some("RUNNING") => Some(ScrubState::Running),
        Some("PAUSED") => Some(ScrubState::Paused),
        Some("COMPLETED") => Some(ScrubState::Finished),
        Some("CANCELLED") => Some(ScrubState::Canceled),
        Some("FAILED") => Some(ScrubState::Failed),
        _ => None,
    };
    let last_finished_at = match state {
        Some(ScrubState::Finished) => {
            let start = check.date.as_deref().and_then(unix_seconds);
            let duration = check.duration.and_then(|d| i64::try_from(d).ok());
            start.zip(duration).and_then(|(s, d)| s.checked_add(d))
        }
        _ => None,
    };
    let progress_percent = match state {
        Some(ScrubState::Running | ScrubState::Paused) => check.progress,
        _ => None,
    };
    Some(ScrubStatus {
        state,
        last_finished_at,
        errors: None,
        progress_percent,
    })
}

/// A share as a volume; its total is used plus free space.
pub(crate) fn share_volume(share: &Share) -> Option<Volume> {
    let name = share.name.clone()?;
    let used = bytes(share.used);
    let total = bytes(
        share
            .used
            .zip(share.free)
            .and_then(|(u, f)| u.checked_add(f)),
    );
    Some(Volume {
        id: format!("{SHARE_PREFIX}{name}"),
        name,
        kind: VolumeKind::Share,
        pool_id: None,
        used_bytes: used,
        total_bytes: total,
        health: None,
        status_text: None,
        raid: None,
        scrub: None,
    })
}

/// Parity, data and cache disks, without empty slots (`DISK_NP`). Disks
/// without a name are skipped.
pub(crate) fn disks(array: &Array, nas_id: &str) -> Vec<Disk> {
    array
        .parities
        .iter()
        .chain(&array.disks)
        .chain(&array.caches)
        .filter(|d| !is_empty_slot(d))
        .filter_map(|d| disk(d, nas_id))
        .collect()
}

fn is_empty_slot(disk: &ArrayDisk) -> bool {
    upper(disk.status.as_deref()).as_deref() == Some("DISK_NP")
}

fn disk(raw: &ArrayDisk, nas_id: &str) -> Option<Disk> {
    let name = raw.name.clone()?;
    let health = match upper(raw.status.as_deref()).as_deref() {
        None => None,
        Some("DISK_OK" | "DISK_NEW") if raw.num_errors.unwrap_or(0) > 0 => Some(Health::Warning),
        Some("DISK_OK" | "DISK_NEW") => Some(Health::Ok),
        Some(_) => Some(Health::Critical),
    };
    Some(Disk {
        id: name.clone(),
        name,
        model: None,
        serial_hash: serial_hash(raw, nas_id),
        temperature_c: raw.temp,
        health,
        smart_text: None,
        power_on_hours: None,
        spinning: raw.is_spinning,
    })
}

/// Hash of the disk id, which Unraid builds from model and serial number.
/// `None` when the id is only the device (`sdb`) or a slot (`slot-…`).
fn serial_hash(raw: &ArrayDisk, nas_id: &str) -> Option<String> {
    let id = local_id(raw.id.as_deref()?).trim();
    if id.is_empty() || Some(id) == raw.device.as_deref() || id.starts_with("slot-") {
        return None;
    }
    Some(hash_serial(nas_id, id))
}

/// Whether the array has a parity disk (an empty slot does not count).
pub(crate) fn has_parity(array: &Array) -> bool {
    array.parities.iter().any(|d| !is_empty_slot(d))
}

/// The parity check as a backup task.
pub(crate) fn parity_task(check: Option<&ParityCheck>) -> BackupTask {
    let last_result = check.and_then(|c| match upper(c.status.as_deref()).as_deref() {
        Some("COMPLETED") => Some(BackupResult::Ok),
        Some("FAILED") => Some(BackupResult::Error),
        Some("RUNNING" | "PAUSED") => Some(BackupResult::Running),
        Some("NEVER_RUN") => Some(BackupResult::Never),
        _ => None,
    });
    BackupTask {
        id: PARITY_TASK_ID.into(),
        name: "Paritätsprüfung".into(),
        kind: BackupKind::Parity,
        last_run: check.and_then(|c| c.date.as_deref()).and_then(unix_seconds),
        last_result,
        next_run: None,
    }
}

/// A notification as alert; the text is the subject, else the title.
/// Without an importance Unraid knows, the alert counts as info.
pub(crate) fn alert(raw: &Notification) -> Option<Alert> {
    let text = raw.subject.clone().or_else(|| raw.title.clone())?;
    let level = match upper(raw.importance.as_deref()).as_deref() {
        Some("ALERT") => AlertLevel::Critical,
        Some("WARNING") => AlertLevel::Warning,
        _ => AlertLevel::Info,
    };
    Some(Alert {
        level,
        text,
        since: raw.timestamp.as_deref().and_then(unix_seconds),
    })
}

/// The first UPS. Unraid reads it with apcupsd (`apcaccess`): the status
/// is a list of words such as `ONLINE`, `ONBATT`, `ONLINE REPLACEBATT`,
/// `TRIM ONLINE` or `ONBATT LOWBATT`; NUT forms (`OL CHRG`, `OB DISCHRG`)
/// are read as well. Any other status leaves the state `None`. Unraid
/// passes the remaining runtime of apcupsd (`TIMELEFT : 45.0 Minutes`) as
/// whole minutes, although its schema says seconds.
pub(crate) fn ups(devices: &[UpsDevice]) -> Option<Ups> {
    let device = devices.first()?;
    let status = device
        .status
        .as_deref()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let words: Vec<&str> = status.split_whitespace().collect();
    let on_battery = words
        .iter()
        .any(|w| matches!(*w, "ONBATT" | "ONBATTERY" | "OB" | "LOWBATT" | "LB"));
    let online = words.iter().any(|w| matches!(*w, "ONLINE" | "OL"));
    let state = if on_battery {
        Some(UpsState::OnBattery)
    } else if online {
        Some(UpsState::Online)
    } else {
        None
    };
    let battery = device.battery.as_ref();
    Some(Ups {
        state,
        charge_percent: battery.and_then(|b| b.charge_level),
        runtime_seconds: battery
            .and_then(|b| b.estimated_runtime)
            .and_then(|minutes| minutes.checked_mul(60)),
    })
}

/// A container with its first name (without the leading `/`), else its id.
pub(crate) fn container(raw: &DockerContainer) -> Option<Container> {
    let name = raw
        .names
        .first()
        .map(|n| n.trim().trim_start_matches('/').to_owned())
        .filter(|n| !n.is_empty())
        .or_else(|| raw.id.as_deref().map(|id| local_id(id).to_owned()))?;
    Some(Container {
        name,
        running: upper(raw.state.as_deref()).as_deref() == Some("RUNNING"),
    })
}

/// A past parity check as backup run: it ended at `date` and began
/// `duration` seconds before (without a duration, at `date`). Ok only when
/// completed without errors.
pub(crate) fn parity_run(raw: &ParityCheck) -> Option<BackupRun> {
    let end = unix_seconds(raw.date.as_deref()?)?;
    let duration = raw
        .duration
        .and_then(|d| i64::try_from(d).ok())
        .unwrap_or(0);
    let status = upper(raw.status.as_deref());
    let errors = raw.errors.unwrap_or(0);
    let (result, message) = match status.as_deref() {
        Some("COMPLETED") if errors == 0 => (BackupResult::Ok, None),
        Some("CANCELLED") => (BackupResult::Error, Some("abgebrochen".to_owned())),
        _ if errors > 0 => (BackupResult::Error, Some(format!("{errors} Fehler"))),
        _ => (BackupResult::Error, None),
    };
    Some(BackupRun {
        task_id: PARITY_TASK_ID.into(),
        started_at: end.saturating_sub(duration),
        finished_at: Some(end),
        result,
        message,
    })
}
