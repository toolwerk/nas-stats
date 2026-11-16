//! The history of one NAS as the collector on the NAS (#46) hands it to the
//! apps: raw samples, hourly and daily values and backup runs, as JSON.
//!
//! Texts are stored as in the database: the metric as its name (`disk.temp`),
//! an empty key for a value of the whole NAS, the backup result as its serde
//! name (`ok`). A reader skips rows it does not know, so a newer collector
//! can add metrics without breaking an older app. Events are not part of the
//! export: the app detects them itself.
//!
//! [`crate::HistoryStore::export`] writes it, [`crate::HistoryStore::import`]
//! reads it into the archive of the app.

use nas_model::ProviderKind;
use serde::{Deserialize, Serialize};

/// Version of the export this build writes and reads.
pub const EXPORT_VERSION: u32 = 1;

/// History of one NAS since a point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Export {
    /// [`EXPORT_VERSION`] of the writer.
    pub version: u32,
    /// Id of the NAS at the writer.
    pub nas_id: String,
    /// Time of the export, Unix seconds UTC; the next export starts here.
    pub until: i64,
    /// Raw samples, from the first full hour of raw samples on.
    pub samples: Vec<ExportSample>,
    /// Hourly values before the raw samples, from the first full day of
    /// hourly values on.
    pub hourly: Vec<ExportBucket>,
    /// Daily values of every day.
    pub daily: Vec<ExportBucket>,
    /// Backup runs.
    pub backup_runs: Vec<ExportBackupRun>,
}

/// One NAS of the collector as `GET /v1/nas` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectorNas {
    /// Id of the NAS in the configuration of the collector.
    pub id: String,
    pub provider: ProviderKind,
    /// Name and model as the NAS reported them last; `None` before.
    pub name: Option<String>,
    pub model: Option<String>,
    pub base_url: String,
    /// Fingerprint of the certificate of the NAS, `AB:CD:…`.
    pub fingerprint: String,
    /// Time of the newest snapshot in the archive, Unix seconds UTC.
    pub last_seen: Option<i64>,
    /// No more queries after a rejected login, until the access changes.
    pub blocked: bool,
    /// Error of the last query, as text.
    pub error: Option<String>,
}

/// One raw sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSample {
    /// Name of the metric, e.g. `disk.temp`.
    pub metric: String,
    /// Volume, disk or interface id; empty for a value of the whole NAS.
    pub key: String,
    /// Unix seconds UTC.
    pub at: i64,
    pub value: f64,
}

/// One hourly or daily value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBucket {
    pub metric: String,
    /// Empty for a value of the whole NAS.
    pub key: String,
    /// Start of the hour or day, Unix seconds UTC.
    pub start: i64,
    pub min: f64,
    pub mean: f64,
    pub max: f64,
    /// Number of raw samples in the value.
    pub count: u32,
}

/// One backup run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBackupRun {
    pub task_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// `ok`, `error`, `running` or `never`.
    pub result: String,
    pub message: Option<String>,
}

/// What an import took over: rows written or changed, per kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportCount {
    pub samples: u32,
    pub hourly: u32,
    pub daily: u32,
    pub backup_runs: u32,
}
