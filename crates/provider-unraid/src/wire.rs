//! The GraphQL answers as tolerant Rust types.
//!
//! Every field is optional. A missing field, `null` or a value of the wrong
//! type leaves only that field `None` (lists: empty, a broken entry is
//! skipped). Numbers may come as text (`BigInt`), enums are kept as text and
//! compared without regard to case.

use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer};
use serde_json::Value;

/// Any value, `None` when it does not fit `T`.
pub(crate) fn opt<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// A list; `null` or another type is empty, entries that do not fit `T`
/// are skipped.
pub(crate) fn list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<Value>::deserialize(d)?;
    Ok(match value {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// A number, also sent as text.
pub(crate) fn num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    Ok(value.as_ref().and_then(number).filter(|n| n.is_finite()))
}

/// A whole number of at least 0, also sent as text. Exact up to `u64::MAX`.
pub(crate) fn count<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    Ok(match value {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().and_then(whole)),
        Some(Value::String(s)) => {
            let s = s.trim();
            s.parse::<u64>()
                .ok()
                .or_else(|| s.parse::<f64>().ok().and_then(whole))
        }
        _ => None,
    })
}

/// Text, also sent as a number; empty text is `None`.
pub(crate) fn text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    let text = match value {
        Some(Value::String(s)) => s,
        Some(Value::Number(n)) => n.to_string(),
        _ => return Ok(None),
    };
    let text = text.trim();
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

/// A flag, also sent as text `true`/`false`.
pub(crate) fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    Ok(match value {
        Some(Value::Bool(b)) => Some(b),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// `n` as a whole number of at least 0, `None` for fractions and negatives.
fn whole(n: f64) -> Option<u64> {
    (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= u64::MAX as f64).then_some(n as u64)
}

/// An enum value in upper case, for comparing.
pub(crate) fn upper(value: Option<&str>) -> Option<String> {
    value.map(|v| v.trim().to_ascii_uppercase())
}

/// The local part of a `PrefixedID` (`<server>:<id>`): the text after the
/// first `:`, the whole text without one.
pub(crate) fn local_id(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, local)| local)
}

/// A point in time as Unix seconds: ISO 8601 with offset
/// (`2026-11-10T03:00:00Z`), or a number of Unix seconds (milliseconds
/// above 10^11). Anything else, e.g. a local time without offset, is `None`.
pub(crate) fn unix_seconds(value: &str) -> Option<i64> {
    let value = value.trim();
    if let Ok(timestamp) = value.parse::<jiff::Timestamp>() {
        return Some(timestamp.as_second());
    }
    let n: i64 = value.parse().ok()?;
    Some(if n.abs() >= 100_000_000_000 {
        n / 1000
    } else {
        n
    })
}

// ---------------------------------------------------------------------------
// Answer of the main query.

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct MainData {
    #[serde(deserialize_with = "opt")]
    pub vars: Option<Vars>,
    #[serde(deserialize_with = "opt")]
    pub info: Option<Info>,
    #[serde(deserialize_with = "opt")]
    pub array: Option<Array>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Vars {
    #[serde(deserialize_with = "text")]
    pub name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Info {
    #[serde(deserialize_with = "opt")]
    pub os: Option<Os>,
    #[serde(deserialize_with = "opt")]
    pub system: Option<System>,
    #[serde(deserialize_with = "opt")]
    pub versions: Option<Versions>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Os {
    #[serde(deserialize_with = "text")]
    pub hostname: Option<String>,
    /// Time of the last boot (ISO 8601), not a duration.
    #[serde(deserialize_with = "text")]
    pub uptime: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct System {
    #[serde(deserialize_with = "text")]
    pub manufacturer: Option<String>,
    #[serde(deserialize_with = "text")]
    pub model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Versions {
    #[serde(deserialize_with = "opt")]
    pub core: Option<CoreVersions>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CoreVersions {
    #[serde(deserialize_with = "text")]
    pub unraid: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Array {
    #[serde(deserialize_with = "text")]
    pub state: Option<String>,
    #[serde(deserialize_with = "opt")]
    pub capacity: Option<Capacity>,
    #[serde(deserialize_with = "opt")]
    pub parity_check_status: Option<ParityCheck>,
    #[serde(deserialize_with = "list")]
    pub parities: Vec<ArrayDisk>,
    #[serde(deserialize_with = "list")]
    pub disks: Vec<ArrayDisk>,
    #[serde(deserialize_with = "list")]
    pub caches: Vec<ArrayDisk>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Capacity {
    #[serde(deserialize_with = "opt")]
    pub kilobytes: Option<Kilobytes>,
}

/// Space in kilobytes of 1000 bytes.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Kilobytes {
    #[serde(deserialize_with = "count")]
    pub free: Option<u64>,
    #[serde(deserialize_with = "count")]
    pub used: Option<u64>,
    #[serde(deserialize_with = "count")]
    pub total: Option<u64>,
}

/// A parity check: the current one (`parityCheckStatus`, `date` is its
/// start) or a past one (`parityHistory`, `date` is its end).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ParityCheck {
    #[serde(deserialize_with = "text")]
    pub date: Option<String>,
    /// Seconds.
    #[serde(deserialize_with = "count")]
    pub duration: Option<u64>,
    #[serde(deserialize_with = "text")]
    pub status: Option<String>,
    /// Percent.
    #[serde(deserialize_with = "num")]
    pub progress: Option<f64>,
    #[serde(deserialize_with = "count")]
    pub errors: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ArrayDisk {
    /// `PrefixedID`, holds model and serial number: never log it.
    #[serde(deserialize_with = "text")]
    pub id: Option<String>,
    #[serde(deserialize_with = "text")]
    pub name: Option<String>,
    #[serde(deserialize_with = "text")]
    pub device: Option<String>,
    #[serde(deserialize_with = "text")]
    pub status: Option<String>,
    #[serde(deserialize_with = "num")]
    pub temp: Option<f64>,
    #[serde(deserialize_with = "count")]
    pub num_errors: Option<u64>,
    #[serde(deserialize_with = "flag")]
    pub is_spinning: Option<bool>,
}

// ---------------------------------------------------------------------------
// Answers of the optional queries.

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct MetricsData {
    #[serde(deserialize_with = "opt")]
    pub metrics: Option<Metrics>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Metrics {
    #[serde(deserialize_with = "opt")]
    pub cpu: Option<CpuMetrics>,
    #[serde(deserialize_with = "opt")]
    pub memory: Option<MemoryMetrics>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct CpuMetrics {
    #[serde(deserialize_with = "num")]
    pub percent_total: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct MemoryMetrics {
    /// Bytes.
    #[serde(deserialize_with = "count")]
    pub total: Option<u64>,
    #[serde(deserialize_with = "num")]
    pub percent_total: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SharesData {
    #[serde(deserialize_with = "list")]
    pub shares: Vec<Share>,
}

/// A user share, space in kilobytes of 1000 bytes.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Share {
    #[serde(deserialize_with = "text")]
    pub name: Option<String>,
    #[serde(deserialize_with = "count")]
    pub free: Option<u64>,
    #[serde(deserialize_with = "count")]
    pub used: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct NotificationsData {
    #[serde(deserialize_with = "opt")]
    pub notifications: Option<Notifications>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Notifications {
    #[serde(deserialize_with = "list")]
    pub list: Vec<Notification>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Notification {
    #[serde(deserialize_with = "text")]
    pub title: Option<String>,
    #[serde(deserialize_with = "text")]
    pub subject: Option<String>,
    #[serde(deserialize_with = "text")]
    pub importance: Option<String>,
    #[serde(deserialize_with = "text")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct UpsData {
    #[serde(deserialize_with = "list")]
    pub ups_devices: Vec<UpsDevice>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct UpsDevice {
    #[serde(deserialize_with = "text")]
    pub status: Option<String>,
    #[serde(deserialize_with = "opt")]
    pub battery: Option<UpsBattery>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct UpsBattery {
    /// Percent.
    #[serde(deserialize_with = "num")]
    pub charge_level: Option<f64>,
    /// Minutes in fact (see `map::ups`), seconds by the schema.
    #[serde(deserialize_with = "count")]
    pub estimated_runtime: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DockerData {
    #[serde(deserialize_with = "opt")]
    pub docker: Option<Docker>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Docker {
    #[serde(deserialize_with = "list")]
    pub containers: Vec<DockerContainer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DockerContainer {
    #[serde(deserialize_with = "text")]
    pub id: Option<String>,
    #[serde(deserialize_with = "list")]
    pub names: Vec<String>,
    #[serde(deserialize_with = "text")]
    pub state: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ParityHistoryData {
    #[serde(deserialize_with = "list")]
    pub parity_history: Vec<ParityCheck>,
}

/// `data` as `T`; an object always fits, since every field is tolerant.
pub(crate) fn decode<T: DeserializeOwned + Default>(data: Value) -> T {
    serde_json::from_value(data).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_come_as_number_or_text() {
        let k: Kilobytes =
            serde_json::from_str(r#"{"free":"1000","used":2000.0,"total":"x"}"#).expect("kb");
        assert_eq!(k.free, Some(1000));
        assert_eq!(k.used, Some(2000));
        assert_eq!(k.total, None);
        let big: Kilobytes =
            serde_json::from_str(r#"{"free":"18446744073709551615"}"#).expect("big");
        assert_eq!(big.free, Some(u64::MAX));
        let negative: Kilobytes = serde_json::from_str(r#"{"free":-1}"#).expect("neg");
        assert_eq!(negative.free, None);
    }

    #[test]
    fn wrong_types_and_broken_entries_are_skipped() {
        let array: Array = serde_json::from_str(
            r#"{"state":5,"capacity":"x","disks":[1,{"name":"disk1","isSpinning":"false"}],"caches":null}"#,
        )
        .expect("array");
        assert_eq!(array.state.as_deref(), Some("5"));
        assert!(array.capacity.is_none());
        assert_eq!(array.disks.len(), 1);
        assert_eq!(array.disks[0].is_spinning, Some(false));
        assert!(array.caches.is_empty());
    }

    #[test]
    fn local_id_takes_the_part_after_the_first_colon() {
        assert_eq!(local_id("abc:MODEL_SERIAL-A"), "MODEL_SERIAL-A");
        assert_eq!(local_id("abc:x:y"), "x:y");
        assert_eq!(local_id("plain"), "plain");
    }

    #[test]
    fn times_need_an_offset_or_are_unix_seconds() {
        assert_eq!(unix_seconds("2026-11-10T03:00:00Z"), Some(1_794_279_600));
        assert_eq!(
            unix_seconds("2026-11-10T04:00:00+01:00"),
            Some(1_794_279_600)
        );
        assert_eq!(unix_seconds("1794279600"), Some(1_794_279_600));
        assert_eq!(unix_seconds("1794279600000"), Some(1_794_279_600));
        assert_eq!(unix_seconds("2026-11-10 03:00:00"), None);
        assert_eq!(unix_seconds(""), None);
    }
}
