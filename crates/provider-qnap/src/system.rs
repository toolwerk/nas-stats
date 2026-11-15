//! System values: model, firmware, uptime, CPU, memory, temperatures, fans,
//! the overall health and an offered firmware update.
//!
//! Sources, all read-only:
//!
//! - `management/manaRequest.cgi?subfunc=sysinfo&hd=no&multicpu=1`
//! - `management/manaRequest.cgi?subfunc=sysinfo&sysHealth=1`
//! - `sys/sysRequest.cgi?subfunc=firm_update`, at most once per
//!   [`FIRMWARE_INTERVAL`]
//!
//! The UPS stays `None`: QTS lists a connected UPS in
//! `devices/devRequest.cgi?func=getExternalDev`, but no recorded answer
//! (issue #1) holds its state, charge or runtime.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use nas_model::{Alert, AlertLevel, Fan, Health, ProviderError, SystemStatus, Ups};
use roxmltree::Document;

use crate::client::QnapClient;
use crate::transport::Transport;
use crate::xml::{number, parse_qdoc, path_text};

/// How long a firmware update answer is reused. QTS may ask the vendor's
/// update server for it, so it is not fetched on every poll.
pub const FIRMWARE_INTERVAL: Duration = Duration::from_secs(3600);

const MANA_PATH: &str = "management/manaRequest.cgi";
const FIRMWARE_PATH: &str = "sys/sysRequest.cgi";
const SYSINFO_QUERY: &[(&str, &str)] = &[("subfunc", "sysinfo"), ("hd", "no"), ("multicpu", "1")];
const HEALTH_QUERY: &[(&str, &str)] = &[("subfunc", "sysinfo"), ("sysHealth", "1")];
const FIRMWARE_QUERY: &[(&str, &str)] = &[("subfunc", "firm_update")];

/// Upper bound for the fan count a NAS reports, so a broken answer cannot
/// make the parser build millions of fans.
const MAX_FANS: u32 = 64;

/// QTS reports memory in MiB.
const MIB: f64 = 1_048_576.0;

/// Whether an update is offered and its version.
pub(crate) type FirmwareInfo = (Option<bool>, Option<String>);

/// Last firmware answer and when it was fetched.
pub(crate) type FirmwareCache = Mutex<Option<(Instant, FirmwareInfo)>>;

/// The system part of a QNAP snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QnapSystem {
    /// Display model, e.g. `TS-1685`.
    pub model: Option<String>,
    /// Firmware version with build, e.g. `5.2.11 Build 20261023`.
    pub firmware: Option<String>,
    pub hostname: Option<String>,
    pub status: SystemStatus,
    pub fans: Vec<Fan>,
    pub ups: Option<Ups>,
    pub alerts: Vec<Alert>,
}

impl<T: Transport> QnapClient<T> {
    /// Reads the system values, one request after the other.
    ///
    /// An error of the `sysinfo` request is returned. For the health and the
    /// firmware request, `PermissionDenied` or `Parse` leave only their value
    /// empty; any other error is returned.
    pub async fn system(&self) -> Result<QnapSystem, ProviderError> {
        let body = self.get_xml(MANA_PATH, SYSINFO_QUERY).await?;
        let mut system = parse_sysinfo(&body)?;

        if let Some(body) = optional(self.get_xml(MANA_PATH, HEALTH_QUERY).await)? {
            system.alerts.extend(parse_health(&body));
        }

        let (available, version) = self.firmware().await?;
        system.status.update_available = available;
        system.status.update_version = version;
        Ok(system)
    }

    /// The firmware answer, from the cache while it is younger than
    /// [`FIRMWARE_INTERVAL`]. Every answer is kept, also an empty one.
    async fn firmware(&self) -> Result<FirmwareInfo, ProviderError> {
        {
            let cache = self
                .firmware_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some((at, info)) = cache.as_ref()
                && at.elapsed() < FIRMWARE_INTERVAL
            {
                return Ok(info.clone());
            }
        }
        let info = match optional(self.get_xml(FIRMWARE_PATH, FIRMWARE_QUERY).await)? {
            Some(body) => parse_firmware(&body),
            None => (None, None),
        };
        *self
            .firmware_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), info.clone()));
        Ok(info)
    }
}

/// `None` for a part the account may not read or that cannot be read.
fn optional(result: Result<String, ProviderError>) -> Result<Option<String>, ProviderError> {
    match result {
        Ok(body) => Ok(Some(body)),
        Err(ProviderError::PermissionDenied | ProviderError::Parse(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Text below `func/ownContent/root`.
fn sys(doc: &Document<'_>, name: &str) -> Option<String> {
    path_text(doc, &["func", "ownContent", "root", name])
}

fn sys_number(doc: &Document<'_>, name: &str) -> Option<f64> {
    sys(doc, name).as_deref().and_then(number)
}

/// Parses the `sysinfo` answer. Only an answer that is not `QDocRoot` XML
/// is an error; every value that is missing or unreadable stays `None`.
pub(crate) fn parse_sysinfo(body: &str) -> Result<QnapSystem, ProviderError> {
    let doc = parse_qdoc(body)?;

    let model = path_text(&doc, &["model", "displayModelName"])
        .or_else(|| path_text(&doc, &["model", "modelName"]));
    let firmware = path_text(&doc, &["firmware", "version"]).map(|version| {
        match path_text(&doc, &["firmware", "build"]) {
            Some(build) => format!("{version} Build {build}"),
            None => version,
        }
    });
    let hostname = sys(&doc, "server_name").or_else(|| path_text(&doc, &["hostname"]));

    let total = sys_number(&doc, "total_memory");
    let free = sys_number(&doc, "free_memory");
    let memory_percent = match (total, free) {
        (Some(total), Some(free)) if total > 0.0 && (0.0..=total).contains(&free) => {
            Some((total - free) / total * 100.0)
        }
        _ => None,
    };
    let memory_total_bytes = total.filter(|t| *t > 0.0).map(|t| (t * MIB).round() as u64);

    let status = SystemStatus {
        uptime_seconds: uptime(&doc),
        cpu_percent: sys_number(&doc, "cpu_usage"),
        memory_percent,
        memory_total_bytes,
        temperature_c: temperature(&doc, "sys_tempc"),
        cpu_temperature_c: temperature(&doc, "cpu_tempc"),
        update_available: None,
        update_version: None,
    };

    let mut fans = fan_group(&doc, "sysfan_count", "sysfan", "sysfan_fail", "Lüfter");
    fans.extend(fan_group(
        &doc,
        "cpufan_count",
        "cpufan",
        "fan_fail",
        "CPU-Lüfter",
    ));

    Ok(QnapSystem {
        model,
        firmware,
        hostname,
        status,
        fans,
        ups: None,
        alerts: Vec::new(),
    })
}

/// A temperature in °C. QTS writes `-1` (or another negative value) for a
/// sensor it does not have, so negative values are `None`.
fn temperature(doc: &Document<'_>, name: &str) -> Option<f64> {
    sys_number(doc, name).filter(|t| *t >= 0.0)
}

/// Uptime from `uptime_day`, `_hour`, `_min` and `_sec`. A missing part
/// counts as 0; when all four are missing or the sum overflows, `None`.
fn uptime(doc: &Document<'_>) -> Option<u64> {
    let parts = [
        ("uptime_day", 86_400),
        ("uptime_hour", 3_600),
        ("uptime_min", 60),
        ("uptime_sec", 1),
    ];
    let mut seen = false;
    let mut seconds = 0u64;
    for (name, factor) in parts {
        if let Some(value) = sys_number(doc, name).filter(|v| *v >= 0.0) {
            seen = true;
            // A value too large for u64 seconds is unreadable, not a panic.
            seconds = (value as u64)
                .checked_mul(factor)
                .and_then(|part| seconds.checked_add(part))?;
        }
    }
    seen.then_some(seconds)
}

/// Fans `<prefix>1` … `<prefix>n` with `n` from `count`, the failure flag
/// in `<fail>i` and the name `<label> i`. A count above [`MAX_FANS`] is cut
/// to it.
fn fan_group(doc: &Document<'_>, count: &str, prefix: &str, fail: &str, label: &str) -> Vec<Fan> {
    let Some(count) = sys_number(doc, count).filter(|c| *c >= 0.0) else {
        return Vec::new();
    };
    (1..=(count as u32).min(MAX_FANS))
        .map(|i| Fan {
            name: format!("{label} {i}"),
            rpm: sys_number(doc, &format!("{prefix}{i}"))
                .filter(|rpm| *rpm >= 0.0 && *rpm <= f64::from(u32::MAX))
                .map(|rpm| rpm as u32),
            health: sys(doc, &format!("{fail}{i}"))
                .as_deref()
                .and_then(fan_health),
        })
        .collect()
}

/// The failure flag of a fan. Unknown values are `None`.
fn fan_health(flag: &str) -> Option<Health> {
    match flag {
        "0" => Some(Health::Ok),
        "1" => Some(Health::Critical),
        _ => None,
    }
}

/// Parses the `sysHealth` answer: anything but `good` is an alert with the
/// text QTS reports. A text with `error`, `critical` or `abnormal` is
/// critical, every other one a warning.
pub(crate) fn parse_health(body: &str) -> Option<Alert> {
    let doc = parse_qdoc(body).ok()?;
    let status = path_text(&doc, &["func", "ownContent", "sysHealth", "status"])?;
    let lower = status.to_lowercase();
    if lower == "good" {
        return None;
    }
    let level = if ["error", "critical", "abnormal"]
        .iter()
        .any(|word| lower.contains(word))
    {
        AlertLevel::Critical
    } else {
        AlertLevel::Warning
    };
    Some(Alert {
        level,
        text: status,
        since: None,
    })
}

/// Parses the `firm_update` answer. QTS 5.2 puts the offered version in
/// `official/newVersion` and writes `none` when there is no update; older
/// firmware has `newVersion` directly below `ownContent`. Empty, `none` or
/// missing means no update.
pub(crate) fn parse_firmware(body: &str) -> FirmwareInfo {
    let Ok(doc) = parse_qdoc(body) else {
        return (None, None);
    };
    let version = path_text(&doc, &["func", "ownContent", "official", "newVersion"])
        .or_else(|| path_text(&doc, &["func", "ownContent", "newVersion"]))
        .filter(|v| !v.eq_ignore_ascii_case("none"));
    match version {
        Some(version) => (Some(true), Some(version)),
        None => (Some(false), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(status: &str) -> Option<Alert> {
        parse_health(&format!(
            "<QDocRoot><func><ownContent><sysHealth><status><![CDATA[{status}]]></status></sysHealth></ownContent></func></QDocRoot>"
        ))
    }

    #[test]
    fn health_levels() {
        assert_eq!(health("good"), None);
        assert_eq!(health("Good"), None);
        assert_eq!(health(""), None);
        assert_eq!(
            health("warning").map(|a| a.level),
            Some(AlertLevel::Warning)
        );
        assert_eq!(health("error").map(|a| a.level), Some(AlertLevel::Critical));
        assert_eq!(health("Abnormal").map(|a| a.text), Some("Abnormal".into()));
        assert_eq!(
            health("Abnormal").map(|a| a.level),
            Some(AlertLevel::Critical)
        );
        assert_eq!(
            health("unknown").map(|a| a.level),
            Some(AlertLevel::Warning)
        );
    }

    #[test]
    fn fan_flags() {
        assert_eq!(fan_health("0"), Some(Health::Ok));
        assert_eq!(fan_health("1"), Some(Health::Critical));
        assert_eq!(fan_health("-1"), None);
    }

    #[test]
    fn a_fan_without_reading_has_no_rpm() {
        let body = "<QDocRoot><func><ownContent><root><sysfan_count>1</sysfan_count><sysfan1>-1</sysfan1></root></ownContent></func></QDocRoot>";
        let system = parse_sysinfo(body).expect("sysinfo");
        assert_eq!(
            system.fans,
            vec![Fan {
                name: "Lüfter 1".into(),
                rpm: None,
                health: None,
            }]
        );
    }

    #[test]
    fn a_missing_sensor_is_no_temperature() {
        let body = "<QDocRoot><func><ownContent><root><cpu_tempc>-1</cpu_tempc><sys_tempc>-1</sys_tempc></root></ownContent></func></QDocRoot>";
        let status = parse_sysinfo(body).expect("sysinfo").status;
        assert_eq!(status.temperature_c, None);
        assert_eq!(status.cpu_temperature_c, None);
    }

    #[test]
    fn an_overflowing_uptime_is_none() {
        let body = "<QDocRoot><func><ownContent><root><uptime_day>1000000000000000</uptime_day><uptime_hour>1</uptime_hour></root></ownContent></func></QDocRoot>";
        assert_eq!(
            parse_sysinfo(body).expect("sysinfo").status.uptime_seconds,
            None
        );
    }

    #[test]
    fn a_huge_fan_count_is_cut() {
        let body = "<QDocRoot><func><ownContent><root><sysfan_count>4294967295</sysfan_count></root></ownContent></func></QDocRoot>";
        assert_eq!(parse_sysinfo(body).expect("sysinfo").fans.len(), 64);
    }

    #[test]
    fn free_memory_above_total_gives_no_percent() {
        let body = "<QDocRoot><func><ownContent><root><total_memory>100</total_memory><free_memory>120</free_memory></root></ownContent></func></QDocRoot>";
        let status = parse_sysinfo(body).expect("sysinfo").status;
        assert_eq!(status.memory_percent, None);
        assert_eq!(status.memory_total_bytes, Some(100 * 1_048_576));
    }

    #[test]
    fn firmware_without_build_and_older_update_form() {
        let body = "<QDocRoot><firmware><version>5.1.4</version></firmware></QDocRoot>";
        assert_eq!(
            parse_sysinfo(body).expect("sysinfo").firmware.as_deref(),
            Some("5.1.4")
        );
        let old = "<QDocRoot><func><ownContent><newVersion><![CDATA[5.2.0.2782 Build 20240601]]></newVersion></ownContent></func></QDocRoot>";
        assert_eq!(
            parse_firmware(old),
            (Some(true), Some("5.2.0.2782 Build 20240601".into()))
        );
    }

    #[test]
    fn not_qts_is_a_parse_error() {
        assert!(matches!(
            parse_sysinfo("<html/>"),
            Err(ProviderError::Parse(_))
        ));
    }
}
