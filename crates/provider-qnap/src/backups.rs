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
//! the time zone of the NAS, so the next run stays unknown as well.

use nas_model::{BackupKind, BackupResult, BackupTask, ProviderError};
use serde_json::Value;

use crate::client::QnapClient;
use crate::transport::Transport;

const JOBS_PATH: &str = "/cc3/v1/users/system/jobs";

/// The one status of HBS 3 known to mean a successful run.
const STATUS_FINISHED: &str = "Finished";

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
            Ok(json) => Ok(parse_jobs(&json)),
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
/// `enable` stays.
pub(crate) fn parse_jobs(json: &Value) -> Vec<BackupTask> {
    let Some(jobs) = json.get("jobs").and_then(Value::as_array) else {
        return Vec::new();
    };
    if json.get("has_more").and_then(Value::as_bool) == Some(true) {
        log::info!("QNAP Hybrid Backup Sync has more jobs than one page, reading the first");
    }
    jobs.iter().filter_map(parse_job).collect()
}

fn parse_job(job: &Value) -> Option<BackupTask> {
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
            log::info!("QNAP Hybrid Backup Sync status {other:?} is unknown, leaving it open");
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
    })
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
}
