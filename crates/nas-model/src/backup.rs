//! Backup tasks and their past runs.

use serde::{Deserialize, Serialize};

/// Kind of a backup task, named after the vendor feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    /// Synology Hyper Backup.
    HyperBackup,
    /// Synology Active Backup.
    ActiveBackup,
    /// TrueNAS replication.
    Replication,
    /// Cloud sync (TrueNAS, Synology Cloud Sync).
    CloudSync,
    /// Periodic snapshot task.
    Snapshot,
    /// Unraid parity check.
    Parity,
    /// QNAP Hybrid Backup Sync 3.
    Hbs3,
}

/// Result of a backup task or run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupResult {
    Ok,
    Error,
    Running,
    /// The task has never run.
    Never,
}

/// A configured backup task with its last state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupTask {
    pub id: String,
    pub name: String,
    pub kind: BackupKind,
    /// Start of the last run, Unix seconds UTC.
    pub last_run: Option<i64>,
    pub last_result: Option<BackupResult>,
    /// Next scheduled run, Unix seconds UTC.
    pub next_run: Option<i64>,
    /// Longest gap between two scheduled runs in seconds; `None` when the
    /// schedule is unknown (manual start, a plan that is not understood).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<i64>,
}

/// One past run of a backup task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupRun {
    /// Id of the [`BackupTask`] this run belongs to.
    pub task_id: String,
    /// Start of the run, Unix seconds UTC.
    pub started_at: i64,
    /// End of the run, Unix seconds UTC; `None` while it runs or when unknown.
    pub finished_at: Option<i64>,
    pub result: BackupResult,
    /// Message of the vendor about the run.
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_run_survives_json() {
        let run = BackupRun {
            task_id: "b1".into(),
            started_at: 1_699_950_000,
            finished_at: Some(1_699_953_600),
            result: BackupResult::Error,
            message: Some("Demo failure".into()),
        };
        let json = serde_json::to_string(&run).expect("run serializes");
        let back: BackupRun = serde_json::from_str(&json).expect("run deserializes");
        assert_eq!(back, run);
    }

    fn task(interval_secs: Option<i64>) -> BackupTask {
        BackupTask {
            id: "b1".into(),
            name: "Offsite".into(),
            kind: BackupKind::Hbs3,
            last_run: Some(1_699_950_000),
            last_result: Some(BackupResult::Ok),
            next_run: None,
            interval_secs,
        }
    }

    #[test]
    fn a_task_without_interval_reads_and_writes_without_the_field() {
        let json = serde_json::to_string(&task(None)).expect("task serializes");
        assert!(!json.contains("interval_secs"));
        let back: BackupTask = serde_json::from_str(&json).expect("task deserializes");
        assert_eq!(back.interval_secs, None);

        let with = serde_json::to_string(&task(Some(604_800))).expect("task serializes");
        let back: BackupTask = serde_json::from_str(&with).expect("task deserializes");
        assert_eq!(back.interval_secs, Some(604_800));
    }
}
