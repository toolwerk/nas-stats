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
