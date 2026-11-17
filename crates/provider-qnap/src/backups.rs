//! Backup jobs of Hybrid Backup Sync 3 (issue #41).
//!
//! Source, read-only: `GET /cc3/v1/users/system/jobs` on the QTS port, with
//! the QTS session in the header `X-QNAP-SID`. This is the REST interface
//! the web interface of HBS 3 uses; QNAP does not document it.
//!
//! Without HBS 3, with HBS 3 stopped, without the right to read it or with
//! an answer that is no JSON the list is empty, never an error, and nothing
//! is retried: no second login, no second request. `api_server.cgi`, which
//! tells whether the service runs, is not asked; the empty list says enough.
//!
//! Only the status `Finished` has been seen (TS-1685, HBS 3 v26.4.6); it
//! counts as [`BackupResult::Ok`]. Every other status stays unknown
//! (`None`) instead of guessing an error. The schedule is a cron line in
//! the time zone of the NAS, so the next run stays unknown as well; only the
//! longest gap between two runs is read from it (`interval_secs`), for
//! daily, weekly and monthly plans, so a weekly job is not reported overdue
//! after two days. A plan that is not understood leaves the gap unknown.

use std::collections::{BTreeSet, HashSet};
use std::sync::{Mutex, PoisonError};

use nas_model::{BackupKind, BackupResult, BackupTask, ProviderError};
use serde_json::Value;

use crate::client::QnapClient;
use crate::transport::Transport;

const JOBS_PATH: &str = "/cc3/v1/users/system/jobs";

/// The one status of HBS 3 known to mean a successful run.
const STATUS_FINISHED: &str = "Finished";

/// Unknown statuses of HBS 3 already logged by one client, so a job that
/// keeps such a status is logged once, not on every poll.
pub(crate) type SeenStatuses = Mutex<HashSet<String>>;

impl<T: Transport> QnapClient<T> {
    /// Reads the enabled backup jobs of Hybrid Backup Sync 3 with their
    /// last run, in the order HBS 3 lists them.
    ///
    /// Sends exactly one request with the current session (logging in only
    /// when there is none). HTTP 401, 403, any other HTTP error status and
    /// an answer that is no JSON give an empty list and are logged. Timeout,
    /// network errors and a blocked or closed login are returned.
    pub async fn backups(&self) -> Result<Vec<BackupTask>, ProviderError> {
        let sid = self.session_id().await?;
        let headers = [
            ("X-QNAP-SID", sid.as_str()),
            ("Content-Type", "application/json"),
        ];
        let body = match self
            .transport
            .get_text_with_headers(JOBS_PATH, &[], &headers)
            .await
        {
            Ok(body) => body,
            Err(ProviderError::LoginFailed) => {
                log::warn!("QNAP Hybrid Backup Sync answered HTTP 401, no backups");
                return Ok(Vec::new());
            }
            Err(ProviderError::PermissionDenied) => {
                log::warn!("QNAP Hybrid Backup Sync answered HTTP 403, no backups");
                return Ok(Vec::new());
            }
            Err(ProviderError::Http(status)) => {
                log::warn!("QNAP Hybrid Backup Sync answered HTTP {status}, no backups");
                return Ok(Vec::new());
            }
            Err(e) => return Err(e),
        };
        match serde_json::from_str::<Value>(&body) {
            Ok(json) => {
                let mut unknown = BTreeSet::new();
                let tasks = parse_jobs(&json, &mut unknown);
                for status in first_seen(&self.hbs_statuses, unknown) {
                    log::info!(
                        "QNAP Hybrid Backup Sync status {status:?} is unknown, leaving it open"
                    );
                }
                Ok(tasks)
            }
            Err(_) => {
                log::warn!("QNAP Hybrid Backup Sync answered no JSON, no backups");
                Ok(Vec::new())
            }
        }
    }
}

/// The backup jobs of a `jobs` answer. Left out: jobs without id or name,
/// jobs of another kind than `backup` (a restore would warn as overdue) and
/// disabled jobs (they would warn as overdue for good). A job without
/// `enable` stays. Statuses other than `Finished` of the kept jobs go to
/// `unknown`.
pub(crate) fn parse_jobs(json: &Value, unknown: &mut BTreeSet<String>) -> Vec<BackupTask> {
    let Some(jobs) = json.get("jobs").and_then(Value::as_array) else {
        return Vec::new();
    };
    if json.get("has_more").and_then(Value::as_bool) == Some(true) {
        log::info!("QNAP Hybrid Backup Sync has more jobs than one page, reading the first");
    }
    jobs.iter()
        .filter_map(|job| parse_job(job, unknown))
        .collect()
}

/// The statuses of `unknown` not logged before by this client; they count
/// as logged from now on.
fn first_seen(seen: &SeenStatuses, unknown: BTreeSet<String>) -> Vec<String> {
    let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
    unknown
        .into_iter()
        .filter(|status| seen.insert(status.clone()))
        .collect()
}

fn parse_job(job: &Value, unknown: &mut BTreeSet<String>) -> Option<BackupTask> {
    let id = text(job, "_id")?;
    let name = text(job, "name")?;
    if job.get("job_type").and_then(Value::as_str) != Some("backup") {
        return None;
    }
    if job.get("enable").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let statistics = job.get("statistics");
    let last_run = statistics
        .and_then(|s| s.get("start_time"))
        .and_then(Value::as_str)
        .and_then(unix_seconds);
    let status = statistics
        .and_then(|s| s.get("status"))
        .and_then(Value::as_str);
    let last_result = match status {
        Some(STATUS_FINISHED) => Some(BackupResult::Ok),
        Some(other) => {
            unknown.insert(other.to_owned());
            None
        }
        None => None,
    };
    Some(BackupTask {
        id,
        name,
        kind: BackupKind::Hbs3,
        last_run,
        last_result,
        next_run: None,
        interval_secs: schedule_interval(job),
    })
}

const DAY_SECS: i64 = 86_400;

/// Longest gap in seconds between two runs of a job, from its `schedule`.
/// `None` when the list is missing or empty (started by hand, as far as
/// known) or when one entry is not a cron line this module understands.
/// With several entries the smallest gap counts, since any of them runs.
fn schedule_interval(job: &Value) -> Option<i64> {
    let entries = job.get("schedule")?.as_array()?;
    let mut shortest: Option<i64> = None;
    for entry in entries {
        if entry.get("type").and_then(Value::as_str) != Some("cron") {
            return None;
        }
        let gap = cron_interval(entry.get("cron_start")?.as_str()?)?;
        shortest = Some(shortest.map_or(gap, |s| s.min(gap)));
    }
    shortest
}

/// Longest gap in seconds between two runs of a cron line with five fields
/// (minute hour day month weekday). Minute and hour are not evaluated.
/// Understood: every day, a weekday set (`0`, `1,4`, `1-5`; `7` is Sunday)
/// and a day-of-month set (`1`, `1,15`, counted with a month of 31 days).
/// Everything else (a month, both day fields, steps, names) is `None`.
fn cron_interval(line: &str) -> Option<i64> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let [_, _, day, month, weekday] = fields[..] else {
        return None;
    };
    if month != "*" {
        return None;
    }
    let days = match (day, weekday) {
        ("*", "*") => return Some(DAY_SECS),
        ("*", weekday) => {
            let set = cron_set(weekday, 0, 7)?
                .into_iter()
                .map(|d| d % 7)
                .collect();
            longest_gap(&set, 7)
        }
        (day, "*") => longest_gap(&cron_set(day, 1, 31)?, 31),
        _ => return None,
    };
    Some(days * DAY_SECS)
}

/// The values of a cron field made of numbers, lists and ranges.
fn cron_set(field: &str, min: u32, max: u32) -> Option<BTreeSet<u32>> {
    let mut set = BTreeSet::new();
    for part in field.split(',') {
        let (from, to): (u32, u32) = match part.split_once('-') {
            Some((from, to)) => (from.parse().ok()?, to.parse().ok()?),
            None => {
                let value = part.parse().ok()?;
                (value, value)
            }
        };
        if from < min || to > max || from > to {
            return None;
        }
        set.extend(from..=to);
    }
    Some(set)
}

/// Largest distance between neighbours of `set` on a cycle of `period`.
fn longest_gap(set: &BTreeSet<u32>, period: u32) -> i64 {
    let (Some(first), Some(last)) = (set.first(), set.last()) else {
        return i64::from(period);
    };
    let wrap = first + period - last;
    let inner = set
        .iter()
        .zip(set.iter().skip(1))
        .map(|(a, b)| b - a)
        .max()
        .unwrap_or(0);
    i64::from(inner.max(wrap))
}

/// A non-empty, trimmed text field.
fn text(job: &Value, key: &str) -> Option<String> {
    let value = job.get(key)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// ISO 8601 with offset in Unix seconds UTC; `None` when empty or unreadable.
fn unix_seconds(value: &str) -> Option<i64> {
    value
        .trim()
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_start_time_with_offset() {
        assert_eq!(
            unix_seconds("2026-11-17T01:00:03+01:00"),
            Some(1_794_873_603)
        );
        assert_eq!(
            unix_seconds("2026-11-17T00:00:03.250Z"),
            Some(1_794_873_603)
        );
        assert_eq!(unix_seconds(""), None);
        assert_eq!(unix_seconds("gestern"), None);
    }

    #[test]
    fn reads_the_longest_gap_of_a_cron_line() {
        const DAY: i64 = 86_400;
        for (line, days) in [
            ("0 1 * * *", 1),
            ("*/15 * * * *", 1),
            ("30 2 * * 0", 7),
            ("0 3 * * 7", 7),
            ("0 3 * * 1,4", 4),
            ("0 3 * * 1-5", 3),
            ("0 4 1 * *", 31),
            ("0 4 1,15 * *", 17),
        ] {
            assert_eq!(cron_interval(line), Some(days * DAY), "{line}");
        }
        for line in [
            "0 4 1 1 *",
            "0 4 1 * 1",
            "0 4 * * MON",
            "0 4 * *",
            "0 4 * * 8",
        ] {
            assert_eq!(cron_interval(line), None, "{line}");
        }
    }

    #[test]
    fn the_schedule_needs_cron_entries_only() {
        let job = |schedule: &str| -> Value {
            serde_json::from_str(&format!(r#"{{"schedule": {schedule}}}"#)).expect("json")
        };
        let cron = |line: &str| format!(r#"{{"type":"cron","cron_start":"{line}"}}"#);
        assert_eq!(schedule_interval(&job("[]")), None);
        assert_eq!(schedule_interval(&serde_json::json!({})), None);
        assert_eq!(
            schedule_interval(&job(&format!("[{}]", cron("30 2 * * 0")))),
            Some(7 * DAY_SECS)
        );
        assert_eq!(
            schedule_interval(&job(&format!(
                "[{},{}]",
                cron("30 2 * * 0"),
                cron("0 1 * * *")
            ))),
            Some(DAY_SECS)
        );
        assert_eq!(
            schedule_interval(&job(&format!(
                "[{},{{\"type\":\"once\"}}]",
                cron("0 1 * * *")
            ))),
            None
        );
    }

    #[test]
    fn an_unknown_status_is_logged_once_per_client() {
        let json: Value =
            serde_json::from_str(include_str!("../tests/fixtures/hbs_jobs_mixed.json"))
                .expect("fixture");
        let seen = SeenStatuses::default();

        let mut unknown = BTreeSet::new();
        parse_jobs(&json, &mut unknown);
        // Left-out jobs (disabled, restore, without id) do not count.
        assert_eq!(unknown, BTreeSet::from(["Running".to_owned()]));
        assert_eq!(first_seen(&seen, unknown), vec!["Running".to_owned()]);

        let mut again = BTreeSet::new();
        parse_jobs(&json, &mut again);
        assert!(first_seen(&seen, again).is_empty());

        let other = BTreeSet::from(["Failed".to_owned(), "Running".to_owned()]);
        assert_eq!(first_seen(&seen, other), vec!["Failed".to_owned()]);
    }
}
