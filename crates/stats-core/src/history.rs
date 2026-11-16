//! History archive in SQLite.
//!
//! Raw samples are kept for 7 days, hourly values (min, mean, max) for
//! 90 days, daily values without limit. Rolling up and pruning happen on
//! every write, so the archive needs no timer and stays complete after a
//! restart. Hours and days are counted in UTC, so every day has 24 hours.
//!
//! A key of `''` in the tables stands for `None` (a value of the whole NAS);
//! a NULL in a primary key would not be unique.

use std::collections::BTreeSet;
use std::path::Path;

use nas_model::{
    BackupResult, BackupRun, Event, EventKind, Health, Metric, ProviderKind, Sample, Snapshot,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::export::{
    EXPORT_VERSION, Export, ExportBackupRun, ExportBucket, ExportEvent, ExportSample, ImportCount,
};
use crate::forecast::{Forecast, forecast, window_start};

/// Version of the schema this build writes and reads.
pub const SCHEMA_VERSION: i64 = 1;

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
/// Raw samples older than this are deleted.
const RAW_KEEP: i64 = 7 * DAY;
/// Hourly values older than this are deleted.
const HOURLY_KEEP: i64 = 90 * DAY;

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS nas (
    id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    name TEXT,
    model TEXT,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS samples (
    nas_id TEXT NOT NULL,
    metric TEXT NOT NULL,
    key TEXT NOT NULL DEFAULT '',
    at INTEGER NOT NULL,
    value REAL NOT NULL,
    PRIMARY KEY (nas_id, metric, key, at)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS samples_at ON samples (at);
CREATE TABLE IF NOT EXISTS samples_hourly (
    nas_id TEXT NOT NULL,
    metric TEXT NOT NULL,
    key TEXT NOT NULL DEFAULT '',
    start INTEGER NOT NULL,
    min REAL NOT NULL,
    mean REAL NOT NULL,
    max REAL NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (nas_id, metric, key, start)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS samples_hourly_start ON samples_hourly (start);
CREATE TABLE IF NOT EXISTS samples_daily (
    nas_id TEXT NOT NULL,
    metric TEXT NOT NULL,
    key TEXT NOT NULL DEFAULT '',
    start INTEGER NOT NULL,
    min REAL NOT NULL,
    mean REAL NOT NULL,
    max REAL NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (nas_id, metric, key, start)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS events (
    nas_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    level TEXT NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (nas_id, kind, subject, started_at)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS events_open ON events (nas_id, ended_at);
CREATE TABLE IF NOT EXISTS backup_runs (
    nas_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    result TEXT NOT NULL,
    message TEXT,
    PRIMARY KEY (nas_id, task_id, started_at)
) WITHOUT ROWID;
";

/// Error of the history archive.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A stored text this build does not know.
    #[error("unknown value in the history: {0}")]
    Value(String),
    /// The file was written by a newer build.
    #[error("history schema {found} is newer than {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    /// An export this build cannot read, e.g. of another version.
    #[error("export not readable: {0}")]
    Format(String),
}

type Result<T> = std::result::Result<T, HistoryError>;

/// Resolution of stored values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Every single sample.
    Raw,
    /// One value per UTC hour.
    Hourly,
    /// One value per UTC day.
    Daily,
}

impl Resolution {
    /// Length of one bucket in seconds; `None` for raw samples.
    fn bucket_secs(self) -> Option<i64> {
        match self {
            Resolution::Raw => None,
            Resolution::Hourly => Some(HOUR),
            Resolution::Daily => Some(DAY),
        }
    }

    fn table(self) -> &'static str {
        match self {
            Resolution::Raw => "samples",
            Resolution::Hourly => "samples_hourly",
            Resolution::Daily => "samples_daily",
        }
    }
}

/// Time range of a chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// Last 24 hours, every sample.
    Day,
    /// Last 7 days, hourly values.
    Week,
    /// Last 30 days, hourly values.
    Month,
    /// Last 365 days, daily values.
    Year,
}

impl Range {
    /// Length of the range in seconds.
    pub fn span(self) -> i64 {
        match self {
            Range::Day => DAY,
            Range::Week => 7 * DAY,
            Range::Month => 30 * DAY,
            Range::Year => 365 * DAY,
        }
    }

    /// Resolution the range is shown in.
    pub fn resolution(self) -> Resolution {
        match self {
            Range::Day => Resolution::Raw,
            Range::Week | Range::Month => Resolution::Hourly,
            Range::Year => Resolution::Daily,
        }
    }
}

/// One point of a series. For raw samples min, mean and max are the value.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Time of the sample or start of the bucket, Unix seconds UTC.
    pub at: i64,
    pub min: f64,
    pub mean: f64,
    pub max: f64,
    /// Number of raw samples in the point.
    pub count: u32,
}

/// All points of one metric and key.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// Volume, disk or interface id; `None` for values of the whole NAS.
    pub key: Option<String>,
    pub points: Vec<Point>,
}

/// A stored event: something about a NAS that needed attention, from its
/// start to its end. Mirrors the event of the detection (#10) field by
/// field; `kind` is its serde name, e.g. `disk_temperature`.
///
/// An event is identified by `nas_id`, `kind`, `subject` and `started_at`.
/// An empty subject is stored like `None` and reads back as `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub nas_id: String,
    pub kind: String,
    /// Disk, volume, task, fan or alert text; `None` for the whole NAS.
    pub subject: Option<String>,
    /// Highest level while the event was open.
    pub level: Health,
    pub text: String,
    /// Start, Unix seconds UTC.
    pub started_at: i64,
    /// End, Unix seconds UTC; `None` while the event is open.
    pub ended_at: Option<i64>,
}

impl From<&Event> for EventRecord {
    fn from(event: &Event) -> Self {
        Self {
            nas_id: event.nas_id.clone(),
            kind: event.kind.as_str().to_owned(),
            subject: event.subject.clone(),
            level: event.level,
            text: event.text.clone(),
            started_at: event.started_at,
            ended_at: event.ended_at,
        }
    }
}

impl EventRecord {
    /// The event of the detection; a kind this build does not know is
    /// [`HistoryError::Value`].
    pub fn to_event(&self) -> Result<Event> {
        let kind = EventKind::from_name(&self.kind)
            .ok_or_else(|| HistoryError::Value(format!("event kind {:?}", self.kind)))?;
        Ok(Event {
            nas_id: self.nas_id.clone(),
            kind,
            subject: self.subject.clone(),
            level: self.level,
            text: self.text.clone(),
            started_at: self.started_at,
            ended_at: self.ended_at,
        })
    }
}

/// History archive of all NAS in one SQLite database.
pub struct HistoryStore {
    conn: Connection,
}

impl HistoryStore {
    /// Opens or creates the archive in a file (WAL mode) and migrates it.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let _mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        Self::with(conn)
    }

    /// Creates an archive in memory, for tests and previews.
    pub fn in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(mut conn: Connection) -> Result<Self> {
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Schema version stored in the file.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// Stores a snapshot: the NAS, its samples at `observed_at` and the last
    /// run of each backup task. Rolls up the touched hours and days and
    /// prunes old values relative to `now`.
    pub fn write_snapshot(&mut self, snapshot: &Snapshot, now: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        upsert_nas(&tx, snapshot)?;
        write_in(&tx, &samples_of(snapshot), now)?;
        upsert_runs(&tx, &snapshot.nas.id, &runs_of(snapshot))?;
        tx.commit()?;
        Ok(())
    }

    /// Stores samples a NAS delivers itself (history of the vendor). Rolls
    /// up and prunes like [`HistoryStore::write_snapshot`]. For hours whose
    /// raw samples are already pruned, the hourly value is built from the
    /// delivered samples only; a day whose hours are already pruned keeps its
    /// daily value and gets one only when it had none.
    pub fn write_samples(&mut self, samples: &[Sample], now: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        write_in(&tx, samples, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Stores backup runs. A known run gets the new result, except that a
    /// finished run does not go back to running; end and message stay when
    /// the new run does not know them.
    pub fn write_backup_runs(&mut self, nas_id: &str, runs: &[BackupRun]) -> Result<()> {
        let tx = self.conn.transaction()?;
        upsert_runs(&tx, nas_id, runs)?;
        tx.commit()?;
        Ok(())
    }

    /// Backup runs of a NAS started in `[from, to]`, oldest first.
    pub fn backup_runs(&self, nas_id: &str, from: i64, to: i64) -> Result<Vec<BackupRun>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT task_id, started_at, finished_at, result, message FROM backup_runs
             WHERE nas_id = ?1 AND started_at BETWEEN ?2 AND ?3
             ORDER BY started_at, task_id",
        )?;
        let rows = stmt.query_map(params![nas_id, from, to], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut runs = Vec::new();
        for row in rows {
            let (task_id, started_at, finished_at, result, message) = row?;
            runs.push(BackupRun {
                task_id,
                started_at,
                finished_at,
                result: result_from(&result)?,
                message,
            });
        }
        Ok(runs)
    }

    /// Points of one metric and key in a resolution, oldest first. Raw
    /// samples count when taken in `[from, to]`, buckets when they start in
    /// it or contain `from`.
    pub fn points(
        &self,
        nas_id: &str,
        metric: Metric,
        key: Option<&str>,
        resolution: Resolution,
        from: i64,
        to: i64,
    ) -> Result<Vec<Point>> {
        let rows = self.query(
            nas_id,
            metric,
            Some(key.unwrap_or("")),
            resolution,
            from,
            to,
        )?;
        Ok(rows.into_iter().map(|(_, point)| point).collect())
    }

    /// All series of a metric in a range ending at `now`, in the resolution
    /// of the range, sorted by key with `None` first. Empty for an unknown
    /// NAS.
    pub fn history(
        &self,
        nas_id: &str,
        metric: Metric,
        range: Range,
        now: i64,
    ) -> Result<Vec<Series>> {
        let rows = self.query(
            nas_id,
            metric,
            None,
            range.resolution(),
            now - range.span(),
            now,
        )?;
        let mut series: Vec<Series> = Vec::new();
        for (key, point) in rows {
            let key = (!key.is_empty()).then_some(key);
            match series.last_mut() {
                Some(last) if last.key == key => last.points.push(point),
                _ => series.push(Series {
                    key,
                    points: vec![point],
                }),
            }
        }
        Ok(series)
    }

    /// Forecast "full in X days" for one volume from its daily values of the
    /// last 30 days (see [`crate::forecast`]). `Ok(None)` means no trend, or
    /// an unknown NAS or volume, or no known size.
    pub fn forecast(&self, nas_id: &str, volume_id: &str, now: i64) -> Result<Option<Forecast>> {
        let from = window_start(now);
        let key = Some(volume_id);
        let totals = self.points(
            nas_id,
            Metric::VolumeTotal,
            key,
            Resolution::Daily,
            from,
            now,
        )?;
        let Some(total) = totals.last() else {
            return Ok(None);
        };
        let used = self.points(
            nas_id,
            Metric::VolumeUsed,
            key,
            Resolution::Daily,
            from,
            now,
        )?;
        Ok(forecast(&used, total.mean, now))
    }

    /// Stores opened, raised and closed events. A known event (same NAS,
    /// kind, subject and start) gets the new level and text; its end stays
    /// once set.
    pub fn write_events(&mut self, events: &[EventRecord]) -> Result<()> {
        let tx = self.conn.transaction()?;
        upsert_events(&tx, events)?;
        tx.commit()?;
        Ok(())
    }

    /// Events of a NAS that are still open, oldest first. The detection
    /// needs them as input after a restart.
    pub fn open_events(&self, nas_id: &str) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT kind, subject, started_at, level, text FROM events
             WHERE nas_id = ?1 AND ended_at IS NULL
             ORDER BY started_at, kind, subject",
        )?;
        let rows = stmt.query_map(params![nas_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (kind, subject, started_at, level, text) = row?;
            events.push(EventRecord {
                nas_id: nas_id.to_owned(),
                kind,
                subject: (!subject.is_empty()).then_some(subject),
                level: health_from(&level)?,
                text,
                started_at,
                ended_at: None,
            });
        }
        Ok(events)
    }

    /// Events of one NAS (`None`: of all NAS) that were open at some time
    /// in `[from, to]`: started at or before `to` and still open or ended at
    /// or after `from`. Newest start first, then by NAS, kind and subject.
    pub fn events(&self, nas_id: Option<&str>, from: i64, to: i64) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT nas_id, kind, subject, started_at, ended_at, level, text FROM events
             WHERE (?1 IS NULL OR nas_id = ?1)
               AND started_at <= ?3 AND (ended_at IS NULL OR ended_at >= ?2)
             ORDER BY started_at DESC, nas_id, kind, subject",
        )?;
        let rows = stmt.query_map(params![nas_id, from, to], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (nas_id, kind, subject, started_at, ended_at, level, text) = row?;
            events.push(EventRecord {
                nas_id,
                kind,
                subject: (!subject.is_empty()).then_some(subject),
                level: health_from(&level)?,
                text,
                started_at,
                ended_at,
            });
        }
        Ok(events)
    }

    /// Ends an open „unreachable“ event of a NAS at `at`, when a snapshot
    /// read at `at` by another device (#37) shows the NAS answered; one
    /// started after `at` stays. `true` when one ended.
    pub fn close_unreachable(&mut self, nas_id: &str, at: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE events SET ended_at = ?3
             WHERE nas_id = ?1 AND kind = ?2 AND ended_at IS NULL AND started_at <= ?3",
            params![nas_id, EventKind::Unreachable.as_str(), at],
        )?;
        Ok(changed > 0)
    }

    /// Deletes everything stored about one NAS.
    pub fn delete_nas(&mut self, nas_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        for table in [
            "samples",
            "samples_hourly",
            "samples_daily",
            "events",
            "backup_runs",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE nas_id = ?1"),
                params![nas_id],
            )?;
        }
        tx.execute("DELETE FROM nas WHERE id = ?1", params![nas_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Last time the NAS answered, Unix seconds UTC (time of its newest
    /// snapshot); `None` for an unknown NAS.
    pub fn last_seen(&self, nas_id: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT last_seen FROM nas WHERE id = ?1")?;
        let mut rows = stmt.query(params![nas_id])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    /// Name and model of a NAS as it reported them last; `None` for an
    /// unknown NAS.
    pub fn name_and_model(&self, nas_id: &str) -> Result<Option<(Option<String>, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT name, model FROM nas WHERE id = ?1")?;
        let mut rows = stmt.query(params![nas_id])?;
        Ok(match rows.next()? {
            Some(row) => Some((row.get(0)?, row.get(1)?)),
            None => None,
        })
    }

    /// The history of a NAS since `since` (`None`: all of it) for the
    /// collector (#46), until `now`. Each time span comes in the finest
    /// resolution that is complete there:
    ///
    /// - raw samples taken after `since`, from the first full hour of raw
    ///   samples on (an hour that pruning may have cut is not full);
    /// - before that, hourly values that end after `since`, from the first
    ///   full day of hourly values on (likewise);
    /// - daily values that end after `since`, of every day: the app rolls up
    ///   only the days its raw samples touch, so the days of the hourly
    ///   values need theirs too;
    /// - backup runs started after `since` minus 7 days, so a run that
    ///   finished later is sent again with its end.
    ///
    /// An unknown NAS gives an empty export.
    pub fn export(&self, nas_id: &str, since: Option<i64>, now: i64) -> Result<Export> {
        let since = since.unwrap_or(0);
        // One read snapshot: a write in between prunes raw samples and would
        // move the first full hour under the queries.
        let tx = self.conn.unchecked_transaction()?;
        let first = |sql: &str| -> Result<Option<i64>> {
            Ok(self
                .conn
                .query_row(sql, params![nas_id], |row| row.get(0))?)
        };
        // The first hour (day) is complete unless pruning may have cut it.
        let complete = |first: i64, len: i64, keep: i64| {
            let start = bucket(first, len);
            if start >= now - keep {
                start
            } else {
                ceil(first, len)
            }
        };
        let raw_from = first("SELECT MIN(at) FROM samples WHERE nas_id = ?1")?
            .map(|at| complete(at, HOUR, RAW_KEEP));
        let first_hour = first("SELECT MIN(start) FROM samples_hourly WHERE nas_id = ?1")?
            .map(|start| complete(start, DAY, HOURLY_KEEP));
        // Hourly values begin at the day of the raw samples at the latest,
        // so a NAS younger than a day still gets its first hours.
        let hourly_from = match (first_hour, raw_from) {
            (Some(hour), Some(raw)) => Some(hour.min(bucket(raw, DAY))),
            (hour, None) => hour,
            (None, Some(raw)) => Some(bucket(raw, DAY)),
        };

        let mut samples = Vec::new();
        if let Some(raw_from) = raw_from {
            let mut stmt = self.conn.prepare_cached(
                "SELECT metric, key, at, value FROM samples
                 WHERE nas_id = ?1 AND at >= ?2 AND at > ?3
                 ORDER BY metric, key, at",
            )?;
            let rows = stmt.query_map(params![nas_id, raw_from, since], |row| {
                Ok(ExportSample {
                    metric: row.get(0)?,
                    key: row.get(1)?,
                    at: row.get(2)?,
                    value: row.get(3)?,
                })
            })?;
            samples = rows.collect::<rusqlite::Result<_>>()?;
        }
        let hourly = match hourly_from {
            Some(from) => self.export_buckets(
                Resolution::Hourly,
                nas_id,
                from,
                raw_from.unwrap_or(i64::MAX),
                since,
            )?,
            None => Vec::new(),
        };
        // Every daily value, also of days with hourly values: the app rolls
        // up only the days its raw samples touch.
        let daily = self.export_buckets(Resolution::Daily, nas_id, i64::MIN, i64::MAX, since)?;

        let mut stmt = self.conn.prepare_cached(
            "SELECT task_id, started_at, finished_at, result, message FROM backup_runs
             WHERE nas_id = ?1 AND started_at > ?2
             ORDER BY started_at, task_id",
        )?;
        let rows = stmt.query_map(params![nas_id, since.saturating_sub(7 * DAY)], |row| {
            Ok(ExportBackupRun {
                task_id: row.get(0)?,
                started_at: row.get(1)?,
                finished_at: row.get(2)?,
                result: row.get(3)?,
                message: row.get(4)?,
            })
        })?;
        let backup_runs = rows.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        tx.commit()?;

        Ok(Export {
            version: EXPORT_VERSION,
            nas_id: nas_id.to_owned(),
            until: now,
            samples,
            hourly,
            daily,
            backup_runs,
            events: Vec::new(),
        })
    }

    /// One UTC day of a NAS for the transfer to another device (#37): the
    /// hourly and daily values starting in the day, the backup runs started
    /// in it and the events open at some time in it (also those still open),
    /// without raw samples. `day` is any time in the day. `None` when the day
    /// holds nothing.
    pub fn export_day(&self, nas_id: &str, day: i64, now: i64) -> Result<Option<Export>> {
        let from = bucket(day, DAY);
        let to = from + DAY;
        let tx = self.conn.unchecked_transaction()?;
        let hourly = self.export_buckets(Resolution::Hourly, nas_id, from, to, i64::MIN)?;
        let daily = self.export_buckets(Resolution::Daily, nas_id, from, to, i64::MIN)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT task_id, started_at, finished_at, result, message FROM backup_runs
             WHERE nas_id = ?1 AND started_at >= ?2 AND started_at < ?3
             ORDER BY started_at, task_id",
        )?;
        let rows = stmt.query_map(params![nas_id, from, to], |row| {
            Ok(ExportBackupRun {
                task_id: row.get(0)?,
                started_at: row.get(1)?,
                finished_at: row.get(2)?,
                result: row.get(3)?,
                message: row.get(4)?,
            })
        })?;
        let backup_runs: Vec<ExportBackupRun> = rows.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        let mut events: Vec<ExportEvent> = self
            .events(Some(nas_id), from, to - 1)?
            .into_iter()
            .map(|event| ExportEvent {
                kind: event.kind,
                subject: event.subject.unwrap_or_default(),
                started_at: event.started_at,
                ended_at: event.ended_at,
                level: health_str(event.level).to_owned(),
                text: event.text,
            })
            .collect();
        events.reverse();
        tx.commit()?;
        if hourly.is_empty() && daily.is_empty() && backup_runs.is_empty() && events.is_empty() {
            return Ok(None);
        }
        Ok(Some(Export {
            version: EXPORT_VERSION,
            nas_id: nas_id.to_owned(),
            until: now,
            samples: Vec::new(),
            hourly,
            daily,
            backup_runs,
            events,
        }))
    }

    /// The raw samples of a NAS of the 24 hours before `now`, for the
    /// transfer to another device (#37), so its day chart has every sample,
    /// and the backup runs started in the 7 days before `now`, so a run that
    /// ended after its day went over still arrives with its end. `None`
    /// without any of both.
    pub fn export_recent(&self, nas_id: &str, now: i64) -> Result<Option<Export>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT metric, key, at, value FROM samples
             WHERE nas_id = ?1 AND at >= ?2
             ORDER BY metric, key, at",
        )?;
        let rows = stmt.query_map(params![nas_id, now - DAY], |row| {
            Ok(ExportSample {
                metric: row.get(0)?,
                key: row.get(1)?,
                at: row.get(2)?,
                value: row.get(3)?,
            })
        })?;
        let samples: Vec<ExportSample> = rows.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        let mut stmt = self.conn.prepare_cached(
            "SELECT task_id, started_at, finished_at, result, message FROM backup_runs
             WHERE nas_id = ?1 AND started_at >= ?2
             ORDER BY started_at, task_id",
        )?;
        let rows = stmt.query_map(params![nas_id, now - 7 * DAY], |row| {
            Ok(ExportBackupRun {
                task_id: row.get(0)?,
                started_at: row.get(1)?,
                finished_at: row.get(2)?,
                result: row.get(3)?,
                message: row.get(4)?,
            })
        })?;
        let backup_runs: Vec<ExportBackupRun> = rows.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        tx.commit()?;
        if samples.is_empty() && backup_runs.is_empty() {
            return Ok(None);
        }
        Ok(Some(Export {
            version: EXPORT_VERSION,
            nas_id: nas_id.to_owned(),
            until: now,
            samples,
            hourly: Vec::new(),
            daily: Vec::new(),
            backup_runs,
            events: Vec::new(),
        }))
    }

    /// Start of every UTC day of a NAS with a daily value, oldest first.
    pub fn days(&self, nas_id: &str) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT start FROM samples_daily WHERE nas_id = ?1 ORDER BY start",
        )?;
        let rows = stmt.query_map(params![nas_id], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Hourly or daily values starting in `[from, to)` that end after
    /// `since`.
    fn export_buckets(
        &self,
        resolution: Resolution,
        nas_id: &str,
        from: i64,
        to: i64,
        since: i64,
    ) -> Result<Vec<ExportBucket>> {
        let table = resolution.table();
        let len = resolution.bucket_secs().unwrap_or(0);
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT metric, key, start, min, mean, max, count FROM {table}
             WHERE nas_id = ?1 AND start >= ?2 AND start < ?3 AND start + ?4 > ?5
             ORDER BY metric, key, start"
        ))?;
        let rows = stmt.query_map(params![nas_id, from, to, len, since], |row| {
            Ok(ExportBucket {
                metric: row.get(0)?,
                key: row.get(1)?,
                start: row.get(2)?,
                min: row.get(3)?,
                mean: row.get(4)?,
                max: row.get(5)?,
                count: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Takes over an export of the collector (#46) under the id `nas_id`
    /// of the app, whatever id the NAS has at the collector. Daily and
    /// hourly values replace stored ones only when they count more samples;
    /// raw samples and backup runs are written like the app's own, so hours
    /// and days are rolled up again and old values pruned relative to
    /// `now`. Events (only from another device, #37) are written like the
    /// app's own: a known one gets level and text, its end stays once set.
    /// Rows with an unknown metric, result, event kind or level and values
    /// that are not finite are skipped. An export of another version is
    /// [`HistoryError::Format`] and changes nothing.
    pub fn import(&mut self, export: &Export, nas_id: &str, now: i64) -> Result<ImportCount> {
        if export.version != EXPORT_VERSION {
            return Err(HistoryError::Format(format!(
                "version {} instead of {EXPORT_VERSION}",
                export.version
            )));
        }
        let mut count = ImportCount::default();
        let tx = self.conn.transaction()?;
        for (resolution, rows, counter) in [
            (Resolution::Daily, &export.daily, &mut count.daily),
            (Resolution::Hourly, &export.hourly, &mut count.hourly),
        ] {
            let table = resolution.table();
            let mut stmt = tx.prepare_cached(&format!(
                "INSERT INTO {table} (nas_id, metric, key, start, min, mean, max, count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (nas_id, metric, key, start) DO UPDATE SET
                     min = excluded.min,
                     mean = excluded.mean,
                     max = excluded.max,
                     count = excluded.count
                 WHERE excluded.count > {table}.count"
            ))?;
            for row in rows {
                let finite = [row.min, row.mean, row.max].iter().all(|v| v.is_finite());
                if metric_from(&row.metric).is_none() || !finite || row.count == 0 {
                    continue;
                }
                let changed = stmt.execute(params![
                    nas_id, row.metric, row.key, row.start, row.min, row.mean, row.max, row.count
                ])?;
                *counter += u32::try_from(changed).unwrap_or(0);
            }
        }
        // A day whose hours came partly from here and partly from the
        // other device is rolled up again from its hours, so it agrees with
        // them; it replaces the daily value only with as many samples.
        let touched: BTreeSet<(&str, &str, i64)> = export
            .hourly
            .iter()
            .filter(|row| metric_from(&row.metric).is_some())
            .map(|row| {
                (
                    row.metric.as_str(),
                    row.key.as_str(),
                    bucket(row.start, DAY),
                )
            })
            .collect();
        rollup_days(
            &tx,
            touched
                .iter()
                .map(|&(metric, key, day)| (nas_id, metric, key, day)),
            now,
        )?;

        let samples: Vec<Sample> = export
            .samples
            .iter()
            .filter(|row| row.value.is_finite())
            .filter_map(|row| {
                Some(Sample {
                    nas_id: nas_id.to_owned(),
                    metric: metric_from(&row.metric)?,
                    key: (!row.key.is_empty()).then(|| row.key.clone()),
                    at: row.at,
                    value: row.value,
                })
            })
            .collect();
        // Only samples that are new or have another value count; a follow-up
        // import overlaps the last one by an hour.
        {
            let mut stored = tx.prepare_cached(
                "SELECT value FROM samples
                 WHERE nas_id = ?1 AND metric = ?2 AND key = ?3 AND at = ?4",
            )?;
            for sample in samples.iter().filter(|s| s.key.as_deref() != Some("")) {
                let value: Option<f64> = stored
                    .query_row(
                        params![
                            nas_id,
                            sample.metric.as_str(),
                            sample.key.as_deref().unwrap_or(""),
                            sample.at
                        ],
                        |row| row.get(0),
                    )
                    .optional()?;
                if value != Some(sample.value) {
                    count.samples += 1;
                }
            }
        }
        write_in(&tx, &samples, now)?;

        let runs: Vec<BackupRun> = export
            .backup_runs
            .iter()
            .filter_map(|row| {
                Some(BackupRun {
                    task_id: row.task_id.clone(),
                    started_at: row.started_at,
                    finished_at: row.finished_at,
                    result: result_from(&row.result).ok()?,
                    message: row.message.clone(),
                })
            })
            .collect();
        // Only runs that are new or change count.
        {
            let mut stored = tx.prepare_cached(
                "SELECT finished_at, result, message FROM backup_runs
                 WHERE nas_id = ?1 AND task_id = ?2 AND started_at = ?3",
            )?;
            let mut read = |run: &BackupRun| {
                stored
                    .query_row(params![nas_id, run.task_id, run.started_at], |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .optional()
            };
            for run in &runs {
                let before = read(run)?;
                upsert_runs(&tx, nas_id, std::slice::from_ref(run))?;
                if read(run)? != before {
                    count.backup_runs += 1;
                }
            }
        }

        let events: Vec<EventRecord> = export
            .events
            .iter()
            .filter(|row| EventKind::from_name(&row.kind).is_some())
            .filter_map(|row| {
                Some(EventRecord {
                    nas_id: nas_id.to_owned(),
                    kind: row.kind.clone(),
                    subject: (!row.subject.is_empty()).then(|| row.subject.clone()),
                    level: health_from(&row.level).ok()?,
                    text: row.text.clone(),
                    started_at: row.started_at,
                    ended_at: row.ended_at,
                })
            })
            .collect();
        // Only events that are new or change count.
        {
            let mut stored = tx.prepare_cached(
                "SELECT ended_at, level, text FROM events
                 WHERE nas_id = ?1 AND kind = ?2 AND subject = ?3 AND started_at = ?4",
            )?;
            let mut read = |event: &EventRecord| {
                stored
                    .query_row(
                        params![
                            nas_id,
                            event.kind,
                            event.subject.as_deref().unwrap_or(""),
                            event.started_at
                        ],
                        |row| {
                            Ok((
                                row.get::<_, Option<i64>>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()
            };
            // An own event of the same problem with another start that
            // overlaps it (both devices saw it begin) stands for it: the
            // other one is not added beside it, its end closes the own one.
            let mut own_overlapping = tx.prepare_cached(
                "SELECT started_at FROM events
                 WHERE nas_id = ?1 AND kind = ?2 AND subject = ?3 AND started_at <> ?4
                   AND started_at <= ?5 AND (ended_at IS NULL OR ended_at >= ?4)
                 ORDER BY started_at LIMIT 1",
            )?;
            let mut close = tx.prepare_cached(
                "UPDATE events SET ended_at = ?5
                 WHERE nas_id = ?1 AND kind = ?2 AND subject = ?3 AND started_at = ?4
                   AND ended_at IS NULL",
            )?;
            for event in &events {
                let subject = event.subject.as_deref().unwrap_or("");
                let own: Option<i64> = own_overlapping
                    .query_row(
                        params![
                            nas_id,
                            event.kind,
                            subject,
                            event.started_at,
                            event.ended_at.unwrap_or(i64::MAX)
                        ],
                        |row| row.get(0),
                    )
                    .optional()?;
                if read(event)?.is_none()
                    && let Some(own) = own
                {
                    if let Some(end) = event.ended_at.filter(|&end| end >= own) {
                        count.events += u32::try_from(
                            close.execute(params![nas_id, event.kind, subject, own, end])?,
                        )
                        .unwrap_or(0);
                    }
                    continue;
                }
                let before = read(event)?;
                upsert_events(&tx, std::slice::from_ref(event))?;
                if read(event)? != before {
                    count.events += 1;
                }
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// Rows `(key, point)` sorted by key and time. `key: None` means all
    /// keys.
    fn query(
        &self,
        nas_id: &str,
        metric: Metric,
        key: Option<&str>,
        resolution: Resolution,
        from: i64,
        to: i64,
    ) -> Result<Vec<(String, Point)>> {
        let table = resolution.table();
        let (sql, from) = match resolution.bucket_secs() {
            None => (
                format!(
                    "SELECT key, at, value, value, value, 1 FROM {table}
                     WHERE nas_id = ?1 AND metric = ?2 AND (?3 IS NULL OR key = ?3)
                       AND at BETWEEN ?4 AND ?5
                     ORDER BY key, at"
                ),
                from,
            ),
            Some(len) => (
                format!(
                    "SELECT key, start, min, mean, max, count FROM {table}
                     WHERE nas_id = ?1 AND metric = ?2 AND (?3 IS NULL OR key = ?3)
                       AND start BETWEEN ?4 AND ?5
                     ORDER BY key, start"
                ),
                bucket(from, len),
            ),
        };
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![nas_id, metric.as_str(), key, from, to], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Point {
                    at: row.get(1)?,
                    min: row.get(2)?,
                    mean: row.get(3)?,
                    max: row.get(4)?,
                    count: row.get(5)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// Steps the schema up from the stored version to [`SCHEMA_VERSION`].
fn migrate(conn: &mut Connection) -> Result<()> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(HistoryError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    if found == SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction()?;
    if found < 1 {
        tx.execute_batch(SCHEMA_V1)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Start of the bucket of length `len` that contains `at`.
fn bucket(at: i64, len: i64) -> i64 {
    at - at.rem_euclid(len)
}

/// Start of the first bucket of length `len` that begins at or after `at`.
fn ceil(at: i64, len: i64) -> i64 {
    let start = bucket(at, len);
    if start == at { at } else { start + len }
}

/// Every metric, to read a stored name back.
const METRICS: [Metric; 8] = [
    Metric::VolumeUsed,
    Metric::VolumeTotal,
    Metric::DiskTemp,
    Metric::Cpu,
    Metric::Mem,
    Metric::NetRx,
    Metric::NetTx,
    Metric::SysTemp,
];

/// The metric of a stored name; `None` for a name this build does not know.
fn metric_from(name: &str) -> Option<Metric> {
    METRICS.into_iter().find(|metric| metric.as_str() == name)
}

fn upsert_nas(tx: &Transaction, snapshot: &Snapshot) -> Result<()> {
    let nas = &snapshot.nas;
    tx.prepare_cached(
        "INSERT INTO nas (id, provider, name, model, first_seen, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT (id) DO UPDATE SET
             provider = excluded.provider,
             name = COALESCE(excluded.name, nas.name),
             model = COALESCE(excluded.model, nas.model),
             last_seen = MAX(nas.last_seen, excluded.last_seen)",
    )?
    .execute(params![
        nas.id,
        provider_str(nas.provider),
        nas.name,
        nas.model,
        snapshot.observed_at
    ])?;
    Ok(())
}

/// Samples of a snapshot at `observed_at`. Missing values give no sample.
fn samples_of(snapshot: &Snapshot) -> Vec<Sample> {
    let at = snapshot.observed_at;
    let mut out = Vec::new();
    let mut push = |metric: Metric, key: Option<&str>, value: Option<f64>| {
        if let Some(value) = value {
            out.push(Sample {
                nas_id: snapshot.nas.id.clone(),
                metric,
                key: key.map(str::to_owned),
                at,
                value,
            });
        }
    };
    for volume in &snapshot.volumes {
        let key = Some(volume.id.as_str());
        push(Metric::VolumeUsed, key, volume.used_bytes.map(|b| b as f64));
        push(
            Metric::VolumeTotal,
            key,
            volume.total_bytes.map(|b| b as f64),
        );
    }
    for disk in &snapshot.disks {
        push(Metric::DiskTemp, Some(disk.id.as_str()), disk.temperature_c);
    }
    push(Metric::Cpu, None, snapshot.system.cpu_percent);
    push(Metric::Mem, None, snapshot.system.memory_percent);
    push(Metric::SysTemp, None, snapshot.system.temperature_c);
    for interface in &snapshot.interfaces {
        let key = Some(interface.name.as_str());
        push(Metric::NetRx, key, interface.rx_bytes_per_sec);
        push(Metric::NetTx, key, interface.tx_bytes_per_sec);
    }
    out
}

/// The last run of each backup task the snapshot names. Tasks that never
/// ran give no run.
fn runs_of(snapshot: &Snapshot) -> Vec<BackupRun> {
    snapshot
        .backups
        .iter()
        .filter_map(|task| match (task.last_run, task.last_result) {
            (
                Some(started_at),
                Some(result @ (BackupResult::Ok | BackupResult::Error | BackupResult::Running)),
            ) => Some(BackupRun {
                task_id: task.id.clone(),
                started_at,
                finished_at: None,
                result,
                message: None,
            }),
            _ => None,
        })
        .collect()
}

fn upsert_events(tx: &Transaction, events: &[EventRecord]) -> Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO events (nas_id, kind, subject, started_at, ended_at, level, text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (nas_id, kind, subject, started_at) DO UPDATE SET
             ended_at = COALESCE(events.ended_at, excluded.ended_at),
             level = excluded.level,
             text = excluded.text",
    )?;
    for event in events {
        stmt.execute(params![
            event.nas_id,
            event.kind,
            event.subject.as_deref().unwrap_or(""),
            event.started_at,
            event.ended_at,
            health_str(event.level),
            event.text
        ])?;
    }
    Ok(())
}

fn upsert_runs(tx: &Transaction, nas_id: &str, runs: &[BackupRun]) -> Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO backup_runs (nas_id, task_id, started_at, finished_at, result, message)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (nas_id, task_id, started_at) DO UPDATE SET
             result = CASE
                 WHEN excluded.result = 'running' AND backup_runs.result <> 'running'
                 THEN backup_runs.result
                 ELSE excluded.result
             END,
             finished_at = COALESCE(excluded.finished_at, backup_runs.finished_at),
             message = COALESCE(excluded.message, backup_runs.message)",
    )?;
    for run in runs {
        stmt.execute(params![
            nas_id,
            run.task_id,
            run.started_at,
            run.finished_at,
            result_str(run.result),
            run.message
        ])?;
    }
    Ok(())
}

/// Core of every write: stores the raw samples, rebuilds every touched hour
/// from the raw samples and every touched day from the hours, then prunes.
///
/// A rebuilt hour or day replaces a stored one only when it counts at least
/// as many samples: an hour taken over from another device (#37) with more
/// samples than this one has raw is kept, never mixed with its own, so a
/// repeated transfer counts nothing twice. Raw samples of both devices add
/// up: each is stored once by its time.
fn write_in(tx: &Transaction, samples: &[Sample], now: i64) -> Result<()> {
    let mut hours = BTreeSet::new();
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO samples (nas_id, metric, key, at, value)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        // An empty key would read back as `None`, so such samples are dropped.
        let valid = |s: &&Sample| s.value.is_finite() && s.key.as_deref() != Some("");
        for sample in samples.iter().filter(valid) {
            let key = sample.key.as_deref().unwrap_or("");
            insert.execute(params![
                sample.nas_id,
                sample.metric.as_str(),
                key,
                sample.at,
                sample.value
            ])?;
            hours.insert((
                sample.nas_id.as_str(),
                sample.metric.as_str(),
                key,
                bucket(sample.at, HOUR),
            ));
        }
    }

    let mut days = BTreeSet::new();
    {
        let mut hourly = tx.prepare_cached(
            "INSERT INTO samples_hourly
                 (nas_id, metric, key, start, min, mean, max, count)
             SELECT nas_id, metric, key, ?4, MIN(value), AVG(value), MAX(value), COUNT(*)
             FROM samples
             WHERE nas_id = ?1 AND metric = ?2 AND key = ?3 AND at >= ?4 AND at < ?4 + ?5
             GROUP BY nas_id, metric, key
             ON CONFLICT (nas_id, metric, key, start) DO UPDATE SET
                 min = excluded.min,
                 mean = excluded.mean,
                 max = excluded.max,
                 count = excluded.count
             WHERE excluded.count >= samples_hourly.count",
        )?;
        for &(nas_id, metric, key, start) in &hours {
            hourly.execute(params![nas_id, metric, key, start, HOUR])?;
            days.insert((nas_id, metric, key, bucket(start, DAY)));
        }
    }

    rollup_days(
        tx,
        days.iter()
            .map(|&(nas_id, metric, key, day)| (nas_id, metric, key, day)),
        now,
    )?;

    tx.prepare_cached("DELETE FROM samples WHERE at < ?1")?
        .execute(params![now - RAW_KEEP])?;
    tx.prepare_cached("DELETE FROM samples_hourly WHERE start < ?1")?
        .execute(params![now - HOURLY_KEEP])?;
    Ok(())
}

/// Rebuilds the daily values `(nas_id, metric, key, day start)` from their
/// hours; one replaces a stored value only with at least as many samples.
/// A day whose hours are partly pruned already keeps its daily value; it is
/// only filled in when there is none yet.
fn rollup_days<'a>(
    tx: &Transaction,
    days: impl Iterator<Item = (&'a str, &'a str, &'a str, i64)>,
    now: i64,
) -> Result<()> {
    {
        let daily_sql = |conflict: &str| {
            format!(
                "INSERT INTO samples_daily
                     (nas_id, metric, key, start, min, mean, max, count)
                 SELECT nas_id, metric, key, ?4, MIN(min), SUM(mean * count) / SUM(count),
                        MAX(max), SUM(count)
                 FROM samples_hourly
                 WHERE nas_id = ?1 AND metric = ?2 AND key = ?3 AND start >= ?4 AND start < ?4 + ?5
                 GROUP BY nas_id, metric, key
                 ON CONFLICT (nas_id, metric, key, start) DO {conflict}"
            )
        };
        let mut rebuild = tx.prepare_cached(&daily_sql(
            "UPDATE SET
                 min = excluded.min,
                 mean = excluded.mean,
                 max = excluded.max,
                 count = excluded.count
             WHERE excluded.count >= samples_daily.count",
        ))?;
        let mut fill = tx.prepare_cached(&daily_sql("NOTHING"))?;
        for (nas_id, metric, key, start) in days {
            let stmt = if start >= now - HOURLY_KEEP {
                &mut rebuild
            } else {
                &mut fill
            };
            stmt.execute(params![nas_id, metric, key, start, DAY])?;
        }
    }
    Ok(())
}

/// Stored name of a provider, as serde writes it.
fn provider_str(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Qnap => "qnap",
        ProviderKind::Synology => "synology",
        ProviderKind::TrueNas => "true_nas",
        ProviderKind::Unraid => "unraid",
    }
}

/// Stored name of a backup result, as serde writes it.
fn result_str(result: BackupResult) -> &'static str {
    match result {
        BackupResult::Ok => "ok",
        BackupResult::Error => "error",
        BackupResult::Running => "running",
        BackupResult::Never => "never",
    }
}

/// Stored name of a level, as serde writes it.
fn health_str(level: Health) -> &'static str {
    match level {
        Health::Ok => "ok",
        Health::Warning => "warning",
        Health::Critical => "critical",
    }
}

fn health_from(text: &str) -> Result<Health> {
    match text {
        "ok" => Ok(Health::Ok),
        "warning" => Ok(Health::Warning),
        "critical" => Ok(Health::Critical),
        other => Err(HistoryError::Value(format!("level {other:?}"))),
    }
}

fn result_from(text: &str) -> Result<BackupResult> {
    match text {
        "ok" => Ok(BackupResult::Ok),
        "error" => Ok(BackupResult::Error),
        "running" => Ok(BackupResult::Running),
        "never" => Ok(BackupResult::Never),
        other => Err(HistoryError::Value(format!("backup result {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_model::{
        BackupKind, BackupTask, Disk, Nas, NetInterface, SystemStatus, Volume, VolumeKind,
    };

    /// UTC midnight, 2023-11-15.
    const T0: i64 = 1_700_006_400;

    fn snapshot(nas_id: &str, at: i64, step: i64) -> Snapshot {
        let wave = (step % 10) as f64;
        Snapshot {
            nas: Nas {
                id: nas_id.into(),
                provider: ProviderKind::Qnap,
                name: Some("Testgerät".into()),
                base_url: "https://nas.example.invalid".into(),
                model: Some("TS-000".into()),
                firmware: None,
                hostname: None,
            },
            observed_at: at,
            system: SystemStatus {
                cpu_percent: Some(10.0 + wave),
                memory_percent: Some(40.0),
                temperature_c: Some(45.0),
                ..SystemStatus::default()
            },
            ups: None,
            volumes: vec![Volume {
                id: "vol-1".into(),
                name: "Daten".into(),
                kind: VolumeKind::Volume,
                pool_id: None,
                used_bytes: Some(1_000_000 + step as u64),
                total_bytes: Some(4_000_000),
                health: None,
                status_text: None,
                raid: None,
                scrub: None,
            }],
            disks: vec![Disk {
                id: "disk-1".into(),
                name: "Platte 1".into(),
                model: None,
                serial_hash: None,
                temperature_c: Some(30.0 + wave),
                health: None,
                smart_text: None,
                power_on_hours: None,
                spinning: None,
            }],
            fans: vec![],
            backups: vec![],
            containers: vec![],
            alerts: vec![],
            interfaces: vec![NetInterface {
                name: "eth0".into(),
                rx_bytes_per_sec: Some(100.0),
                tx_bytes_per_sec: Some(50.0),
            }],
        }
    }

    fn count(store: &HistoryStore, sql: &str) -> i64 {
        store
            .conn
            .query_row(sql, [], |row| row.get(0))
            .expect("count query runs")
    }

    fn counts(store: &HistoryStore) -> [i64; 5] {
        [
            count(store, "SELECT COUNT(*) FROM nas"),
            count(store, "SELECT COUNT(*) FROM samples"),
            count(store, "SELECT COUNT(*) FROM samples_hourly"),
            count(store, "SELECT COUNT(*) FROM samples_daily"),
            count(store, "SELECT COUNT(*) FROM backup_runs"),
        ]
    }

    fn sample(at: i64, value: f64) -> Sample {
        Sample {
            nas_id: "nas-1".into(),
            metric: Metric::DiskTemp,
            key: Some("disk-1".into()),
            at,
            value,
        }
    }

    #[test]
    fn hundred_days_give_the_resolution_per_range_and_prune() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let step_secs = 30 * 60;
        let end = T0 + 100 * DAY;
        let mut step = 0;
        let mut at = T0;
        while at <= end {
            store
                .write_snapshot(&snapshot("nas-1", at, step), at)
                .expect("snapshot is written");
            step += 1;
            at += step_secs;
        }
        let now = end;

        for metric in [
            Metric::Cpu,
            Metric::Mem,
            Metric::SysTemp,
            Metric::DiskTemp,
            Metric::VolumeUsed,
            Metric::VolumeTotal,
            Metric::NetRx,
            Metric::NetTx,
        ] {
            for (range, expected) in [
                (Range::Day, 49),
                (Range::Week, 169),
                (Range::Month, 721),
                (Range::Year, 101),
            ] {
                let series = store
                    .history("nas-1", metric, range, now)
                    .expect("history is read");
                assert_eq!(series.len(), 1, "{metric:?} {range:?}");
                assert_eq!(
                    series[0].points.len(),
                    expected,
                    "{metric:?} {range:?} points"
                );
            }
        }

        let day = store
            .history("nas-1", Metric::Cpu, Range::Day, now)
            .expect("day is read");
        assert!(day[0].points.iter().all(|p| p.count == 1));
        let week = store
            .history("nas-1", Metric::Cpu, Range::Week, now)
            .expect("week is read");
        assert_eq!(week[0].points[0].at, now - 7 * DAY);
        assert_eq!(week[0].points[0].count, 2);

        assert_eq!(
            count(
                &store,
                &format!("SELECT COUNT(*) FROM samples WHERE at < {}", now - RAW_KEEP)
            ),
            0
        );
        assert_eq!(
            count(
                &store,
                &format!(
                    "SELECT COUNT(*) FROM samples_hourly WHERE start < {}",
                    now - HOURLY_KEEP
                )
            ),
            0
        );
        assert_eq!(
            count(
                &store,
                &format!(
                    "SELECT COUNT(*) FROM samples_daily WHERE start = {T0} AND metric = 'cpu'"
                )
            ),
            1
        );
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM samples_daily WHERE metric = 'cpu'"
            ),
            101
        );
    }

    #[test]
    fn hourly_and_daily_values_match_hand_calculation() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let hour1 = T0 + 10 * HOUR;
        let hour2 = T0 + 11 * HOUR;
        let samples = [
            sample(hour1 + 60, 10.0),
            sample(hour1 + 1_200, 20.0),
            sample(hour1 + 3_000, 60.0),
            sample(hour2 + 300, 50.0),
        ];
        store
            .write_samples(&samples, T0 + DAY)
            .expect("samples are written");

        let hourly = store
            .points(
                "nas-1",
                Metric::DiskTemp,
                Some("disk-1"),
                Resolution::Hourly,
                T0,
                T0 + DAY,
            )
            .expect("hours are read");
        assert_eq!(
            hourly,
            vec![
                Point {
                    at: hour1,
                    min: 10.0,
                    mean: 30.0,
                    max: 60.0,
                    count: 3
                },
                Point {
                    at: hour2,
                    min: 50.0,
                    mean: 50.0,
                    max: 50.0,
                    count: 1
                },
            ]
        );

        let daily = store
            .points(
                "nas-1",
                Metric::DiskTemp,
                Some("disk-1"),
                Resolution::Daily,
                T0 + 5 * HOUR,
                T0 + DAY,
            )
            .expect("days are read");
        // (10 + 20 + 60 + 50) / 4 = 35, weighted by the samples per hour.
        assert_eq!(
            daily,
            vec![Point {
                at: T0,
                min: 10.0,
                mean: 35.0,
                max: 60.0,
                count: 4
            }]
        );
    }

    #[test]
    fn late_samples_keep_a_daily_value_whose_hours_are_pruned() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        store
            .write_samples(&[sample(T0 + HOUR, 10.0), sample(T0 + 2 * HOUR, 20.0)], T0)
            .expect("samples are written");
        let now = T0 + 120 * DAY;
        store
            .write_samples(&[], now)
            .expect("old values are pruned");
        let day = |store: &HistoryStore, start: i64| {
            store
                .points(
                    "nas-1",
                    Metric::DiskTemp,
                    Some("disk-1"),
                    Resolution::Daily,
                    start,
                    start,
                )
                .expect("days are read")
        };
        let before = day(&store, T0);
        assert_eq!(before[0].count, 2);

        // A late sample for that day leaves its daily value as it was.
        store
            .write_samples(&[sample(T0 + 3 * HOUR, 90.0)], now)
            .expect("late sample is written");
        assert_eq!(day(&store, T0), before);

        // A day without a daily value yet is filled in.
        store
            .write_samples(&[sample(T0 + DAY + HOUR, 40.0)], now)
            .expect("late sample is written");
        let filled = day(&store, T0 + DAY);
        assert_eq!(filled.len(), 1);
        assert_eq!(filled[0].mean, 40.0);
    }

    #[test]
    fn empty_key_gives_no_row() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut snap = snapshot("nas-1", T0, 0);
        snap.interfaces[0].name = String::new();
        store
            .write_snapshot(&snap, T0)
            .expect("snapshot is written");
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM samples WHERE metric LIKE 'net.%'"
            ),
            0
        );
    }

    #[test]
    fn same_snapshot_twice_changes_no_count() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut snap = snapshot("nas-1", T0, 0);
        snap.backups.push(BackupTask {
            id: "b1".into(),
            name: "Nachts".into(),
            kind: BackupKind::Hbs3,
            last_run: Some(T0 - HOUR),
            last_result: Some(BackupResult::Ok),
            next_run: None,
        });
        store.write_snapshot(&snap, T0).expect("first write");
        let first = counts(&store);
        store.write_snapshot(&snap, T0).expect("second write");
        assert_eq!(counts(&store), first);
        assert_eq!(first, [1, 8, 8, 8, 1]);
    }

    #[test]
    fn missing_and_nan_values_give_no_row_and_key_none_stays_none() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut snap = snapshot("nas-1", T0, 0);
        snap.system.cpu_percent = None;
        snap.system.memory_percent = Some(f64::NAN);
        snap.disks[0].temperature_c = Some(f64::INFINITY);
        store
            .write_snapshot(&snap, T0)
            .expect("snapshot is written");

        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM samples WHERE metric IN ('cpu', 'mem', 'disk.temp')"
            ),
            0
        );
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM samples_hourly WHERE metric IN ('cpu', 'mem', 'disk.temp')"
            ),
            0
        );
        let temp = store
            .history("nas-1", Metric::SysTemp, Range::Day, T0)
            .expect("history is read");
        assert_eq!(temp.len(), 1);
        assert_eq!(temp[0].key, None);
        let rx = store
            .history("nas-1", Metric::NetRx, Range::Day, T0)
            .expect("history is read");
        assert_eq!(rx[0].key.as_deref(), Some("eth0"));
    }

    #[test]
    fn series_are_sorted_by_key_with_none_first() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut samples = Vec::new();
        for key in [Some("b"), None, Some("a")] {
            samples.push(Sample {
                nas_id: "nas-1".into(),
                metric: Metric::NetRx,
                key: key.map(str::to_owned),
                at: T0,
                value: 1.0,
            });
        }
        store.write_samples(&samples, T0).expect("samples written");
        let keys: Vec<_> = store
            .history("nas-1", Metric::NetRx, Range::Week, T0)
            .expect("history is read")
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert_eq!(keys, vec![None, Some("a".into()), Some("b".into())]);
        assert!(
            store
                .history("unknown", Metric::NetRx, Range::Week, T0)
                .expect("history is read")
                .is_empty()
        );
    }

    #[test]
    fn running_run_from_snapshot_is_completed_by_backup_runs() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut snap = snapshot("nas-1", T0, 0);
        let started = T0 - HOUR;
        snap.backups = vec![
            BackupTask {
                id: "b1".into(),
                name: "Nachts".into(),
                kind: BackupKind::Hbs3,
                last_run: Some(started),
                last_result: Some(BackupResult::Running),
                next_run: None,
            },
            BackupTask {
                id: "b2".into(),
                name: "Nie gelaufen".into(),
                kind: BackupKind::Hbs3,
                last_run: Some(started),
                last_result: Some(BackupResult::Never),
                next_run: None,
            },
            BackupTask {
                id: "b3".into(),
                name: "Ohne Zeit".into(),
                kind: BackupKind::Hbs3,
                last_run: None,
                last_result: Some(BackupResult::Ok),
                next_run: None,
            },
        ];
        store
            .write_snapshot(&snap, T0)
            .expect("snapshot is written");
        let runs = store
            .backup_runs("nas-1", T0 - DAY, T0)
            .expect("runs are read");
        assert_eq!(
            runs,
            vec![BackupRun {
                task_id: "b1".into(),
                started_at: started,
                finished_at: None,
                result: BackupResult::Running,
                message: None,
            }]
        );

        let done = BackupRun {
            task_id: "b1".into(),
            started_at: started,
            finished_at: Some(T0 - 60),
            result: BackupResult::Ok,
            message: Some("Fertig".into()),
        };
        store
            .write_backup_runs("nas-1", std::slice::from_ref(&done))
            .expect("runs are written");
        // A later snapshot with the same last run keeps end and message.
        // A stale `Running` does not undo the finished run.
        for result in [BackupResult::Ok, BackupResult::Running] {
            snap.backups[0].last_result = Some(result);
            store
                .write_snapshot(&snap, T0)
                .expect("snapshot is written");
            let runs = store
                .backup_runs("nas-1", T0 - DAY, T0)
                .expect("runs are read");
            assert_eq!(runs, vec![done.clone()], "{result:?}");
        }
    }

    #[test]
    fn stale_running_snapshot_keeps_ok_without_end() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut snap = snapshot("nas-1", T0, 0);
        snap.backups = vec![BackupTask {
            id: "b1".into(),
            name: "Nachts".into(),
            kind: BackupKind::Hbs3,
            last_run: Some(T0 - HOUR),
            last_result: Some(BackupResult::Running),
            next_run: None,
        }];
        for result in [
            BackupResult::Running,
            BackupResult::Ok,
            BackupResult::Running,
        ] {
            snap.backups[0].last_result = Some(result);
            store
                .write_snapshot(&snap, T0)
                .expect("snapshot is written");
        }
        let runs = store
            .backup_runs("nas-1", T0 - DAY, T0)
            .expect("runs are read");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].result, BackupResult::Ok);
    }

    #[test]
    fn new_store_has_schema_version_one() {
        let store = HistoryStore::in_memory().expect("store opens");
        assert_eq!(store.schema_version().expect("version is read"), 1);
    }

    #[test]
    fn reopened_file_keeps_version_and_data() {
        let path = std::env::temp_dir().join(format!(
            "nas-stats-history-test-{}.sqlite",
            std::process::id()
        ));
        let remove = || {
            for suffix in ["", "-wal", "-shm"] {
                let mut file = path.clone().into_os_string();
                file.push(suffix);
                let _ = std::fs::remove_file(file);
            }
        };
        remove();
        {
            let mut store = HistoryStore::open(&path).expect("file opens");
            store
                .write_snapshot(&snapshot("nas-1", T0, 0), T0)
                .expect("snapshot is written");
        }
        let store = HistoryStore::open(&path).expect("file opens again");
        assert_eq!(store.schema_version().expect("version is read"), 1);
        let cpu = store
            .history("nas-1", Metric::Cpu, Range::Day, T0)
            .expect("history is read");
        assert_eq!(cpu[0].points.len(), 1);
        drop(store);
        remove();
    }

    #[test]
    fn newer_schema_is_refused() {
        let mut conn = Connection::open_in_memory().expect("connection opens");
        conn.pragma_update(None, "user_version", 2)
            .expect("version is set");
        assert!(matches!(
            migrate(&mut conn),
            Err(HistoryError::SchemaTooNew {
                found: 2,
                supported: 1
            })
        ));
    }

    #[test]
    fn delete_nas_clears_only_that_nas() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        store
            .write_snapshot(&snapshot("nas-1", T0, 0), T0)
            .expect("first NAS");
        store
            .write_snapshot(&snapshot("nas-2", T0, 0), T0)
            .expect("second NAS");
        store
            .write_events(&[event("nas-1", Some("disk-1"), T0, None)])
            .expect("event is written");
        store.delete_nas("nas-1").expect("NAS is deleted");

        for table in [
            "samples",
            "samples_hourly",
            "samples_daily",
            "events",
            "backup_runs",
        ] {
            assert_eq!(
                count(
                    &store,
                    &format!("SELECT COUNT(*) FROM {table} WHERE nas_id = 'nas-1'")
                ),
                0,
                "{table}"
            );
        }
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM nas WHERE id = 'nas-1'"),
            0
        );
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM samples WHERE nas_id = 'nas-2'"
            ),
            8
        );
        assert_eq!(count(&store, "SELECT COUNT(*) FROM nas"), 1);
    }

    fn event(
        nas_id: &str,
        subject: Option<&str>,
        started_at: i64,
        ended_at: Option<i64>,
    ) -> EventRecord {
        EventRecord {
            nas_id: nas_id.into(),
            kind: "disk_temperature".into(),
            subject: subject.map(str::to_owned),
            level: Health::Warning,
            text: "Platte 1 bei 50 °C".into(),
            started_at,
            ended_at,
        }
    }

    #[test]
    fn events_open_raise_close_and_reload() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let opened = event("nas-1", Some("disk-1"), T0, None);
        let ups = EventRecord {
            kind: "ups_on_battery".into(),
            subject: None,
            level: Health::Critical,
            text: "USV auf Batterie".into(),
            ..event("nas-1", None, T0 + 60, None)
        };
        let other = event("nas-2", Some("disk-1"), T0, None);
        let done = event("nas-1", Some("disk-2"), T0 - DAY, Some(T0 - HOUR));
        store
            .write_events(&[opened.clone(), ups.clone(), other, done])
            .expect("events are written");
        assert_eq!(
            store.open_events("nas-1").expect("open events"),
            vec![opened.clone(), ups.clone()]
        );

        // Raised: same key and start, new level and text.
        let raised = EventRecord {
            level: Health::Critical,
            text: "Platte 1 bei 56 °C".into(),
            ..opened.clone()
        };
        store
            .write_events(std::slice::from_ref(&raised))
            .expect("raise is written");
        assert_eq!(
            store.open_events("nas-1").expect("open events"),
            vec![raised.clone(), ups.clone()]
        );

        // Closed: the end is set and stays when the event is written again.
        let closed = EventRecord {
            ended_at: Some(T0 + HOUR),
            ..raised.clone()
        };
        store.write_events(&[closed]).expect("close is written");
        store
            .write_events(&[raised])
            .expect("stale open is written");
        assert_eq!(store.open_events("nas-1").expect("open events"), vec![ups]);
        assert_eq!(
            count(&store, "SELECT COUNT(*) FROM events WHERE nas_id = 'nas-1'"),
            3
        );
    }

    #[test]
    fn events_in_a_range_of_one_or_all_nas() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        // Ended before the range.
        let before = event("nas-1", Some("disk-1"), T0 - 2 * DAY, Some(T0 - DAY - 1));
        // Ended on the first second of the range.
        let edge = event("nas-1", Some("disk-2"), T0 - 2 * DAY, Some(T0 - DAY));
        // Still open, started long before.
        let open = event("nas-1", Some("disk-3"), T0 - 10 * DAY, None);
        // Started on the last second of the range.
        let last = event("nas-2", None, T0, None);
        // Started after the range.
        let after = event("nas-1", Some("disk-4"), T0 + 1, None);
        // Same start, other NAS: sorted by NAS.
        let twin = event("nas-0", Some("disk-1"), T0, Some(T0 + HOUR));
        store
            .write_events(&[
                before,
                edge.clone(),
                open.clone(),
                last.clone(),
                after,
                twin.clone(),
            ])
            .expect("events are written");

        assert_eq!(
            store.events(None, T0 - DAY, T0).expect("events"),
            vec![twin, last, edge.clone(), open.clone()]
        );
        assert_eq!(
            store.events(Some("nas-1"), T0 - DAY, T0).expect("events"),
            vec![edge, open]
        );
        assert!(
            store
                .events(Some("nas-9"), T0 - DAY, T0)
                .expect("events")
                .is_empty()
        );
    }

    #[test]
    fn event_records_and_events_convert_both_ways() {
        let record = event("nas-1", Some("disk-1"), T0, Some(T0 + HOUR));
        let event = record.to_event().expect("known kind");
        assert_eq!(event.kind, EventKind::DiskTemperature);
        assert_eq!(event.subject.as_deref(), Some("disk-1"));
        assert_eq!(event.ended_at, Some(T0 + HOUR));
        assert_eq!(EventRecord::from(&event), record);

        let unknown = EventRecord {
            kind: "later".into(),
            ..record
        };
        assert!(matches!(unknown.to_event(), Err(HistoryError::Value(_))));
    }

    #[test]
    fn reopened_file_keeps_open_events() {
        let path = std::env::temp_dir().join(format!(
            "nas-stats-events-test-{}.sqlite",
            std::process::id()
        ));
        let remove = || {
            for suffix in ["", "-wal", "-shm"] {
                let mut file = path.clone().into_os_string();
                file.push(suffix);
                let _ = std::fs::remove_file(file);
            }
        };
        remove();
        let opened = event("nas-1", Some("disk-1"), T0, None);
        {
            let mut store = HistoryStore::open(&path).expect("file opens");
            store
                .write_events(std::slice::from_ref(&opened))
                .expect("event is written");
        }
        let store = HistoryStore::open(&path).expect("file opens again");
        assert_eq!(
            store.open_events("nas-1").expect("open events"),
            vec![opened]
        );
        drop(store);
        remove();
    }

    /// Collector archive of `nas_id` with a snapshot every 30 minutes from
    /// `T0` to `end`, and one backup run per day.
    fn collector(nas_id: &str, end: i64) -> HistoryStore {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let mut step = 0;
        let mut at = T0;
        while at <= end {
            let mut snap = snapshot(nas_id, at, step);
            if at % DAY == 0 {
                snap.backups.push(BackupTask {
                    id: "b1".into(),
                    name: "Nachts".into(),
                    kind: BackupKind::Hbs3,
                    last_run: Some(at),
                    last_result: Some(BackupResult::Ok),
                    next_run: None,
                });
            }
            store
                .write_snapshot(&snap, at)
                .expect("snapshot is written");
            step += 1;
            at += 30 * 60;
        }
        store
    }

    const ALL_METRICS: [Metric; 8] = METRICS;

    #[test]
    fn export_and_import_give_the_same_history_under_the_app_id() {
        let end = T0 + 100 * DAY;
        let source = collector("col-1", end);
        let export = source.export("col-1", None, end).expect("export");
        assert_eq!(export.version, EXPORT_VERSION);
        assert_eq!(export.nas_id, "col-1");
        assert_eq!(export.until, end);
        // Raw samples from the first full hour on, hourly values before.
        let raw_from = export.samples.iter().map(|s| s.at).min().expect("raw");
        assert_eq!(raw_from % HOUR, 0);
        assert!(export.hourly.iter().all(|h| h.start < raw_from));
        assert!(
            export
                .hourly
                .iter()
                .all(|h| h.start % DAY == 0 || h.start >= T0 + 10 * DAY)
        );

        let json = serde_json::to_string(&export).expect("export serializes");
        let back: Export = serde_json::from_str(&json).expect("export parses");
        let mut app = HistoryStore::in_memory().expect("store opens");
        let count = app.import(&back, "app-7", end).expect("import");
        assert_eq!(count.samples as usize, export.samples.len());
        assert_eq!(count.hourly as usize, export.hourly.len());
        assert_eq!(count.daily as usize, export.daily.len());
        assert_eq!(count.backup_runs, 101);

        for metric in ALL_METRICS {
            for range in [Range::Day, Range::Week, Range::Month, Range::Year] {
                let mine = app.history("app-7", metric, range, end).expect("app");
                let theirs = source.history("col-1", metric, range, end).expect("source");
                assert!(!mine.is_empty(), "{metric:?} {range:?}");
                assert_eq!(mine, theirs, "{metric:?} {range:?}");
            }
        }
        assert_eq!(
            app.backup_runs("app-7", T0, end).expect("runs"),
            source.backup_runs("col-1", T0, end).expect("runs")
        );
        assert!(
            app.history("col-1", Metric::Cpu, Range::Day, end)
                .expect("history")
                .is_empty()
        );

        // A second import of the same export changes nothing.
        let before = counts(&app);
        let again = app.import(&back, "app-7", end).expect("import again");
        assert_eq!(counts(&app), before);
        assert_eq!(again, ImportCount::default());
    }

    #[test]
    fn export_since_sends_only_what_is_newer() {
        let end = T0 + 20 * DAY;
        let source = collector("col-1", end);
        let since = end - 2 * HOUR - 60;
        let export = source.export("col-1", Some(since), end).expect("export");
        // Two hours and a minute: 5 snapshots of 8 values each.
        assert_eq!(export.samples.len(), 5 * 8);
        assert!(export.samples.iter().all(|s| s.at > since));
        assert!(export.hourly.is_empty());
        // The day of `since` and today, 8 values each.
        assert!(export.daily.iter().all(|d| d.start + DAY > since));
        assert_eq!(export.daily.len(), 2 * 8);
        // Backup runs from 7 days before `since` on, one each midnight.
        assert!(
            export
                .backup_runs
                .iter()
                .all(|r| r.started_at > since - 7 * DAY)
        );
        assert_eq!(export.backup_runs.len(), 8);
    }

    #[test]
    fn young_nas_sends_all_its_raw_samples() {
        let mut source = HistoryStore::in_memory().expect("store opens");
        let start = T0 + 12 * HOUR + 3 * 60;
        for step in 0..4 {
            let at = start + step * 30 * 60;
            source
                .write_snapshot(&snapshot("col-1", at, step), at)
                .expect("snapshot is written");
        }
        let now = start + 2 * HOUR;
        let export = source.export("col-1", None, now).expect("export");
        // Nothing is pruned yet, so the first hour is complete as raw samples.
        assert_eq!(export.samples.len(), 4 * 8);
        assert!(export.hourly.is_empty());

        let mut app = HistoryStore::in_memory().expect("store opens");
        app.import(&export, "app-1", now).expect("import");
        for range in [Range::Day, Range::Week, Range::Year] {
            assert_eq!(
                app.history("app-1", Metric::Cpu, range, now).expect("app"),
                source
                    .history("col-1", Metric::Cpu, range, now)
                    .expect("source"),
                "{range:?}"
            );
        }
    }

    #[test]
    fn unknown_nas_gives_an_empty_export() {
        let source = collector("col-1", T0 + DAY);
        let export = source.export("other", None, T0 + DAY).expect("export");
        assert_eq!(export.nas_id, "other");
        assert!(export.samples.is_empty());
        assert!(export.hourly.is_empty());
        assert!(export.daily.is_empty());
        assert!(export.backup_runs.is_empty());
    }

    fn bucket_row(metric: &str, start: i64, mean: f64, count: u32) -> ExportBucket {
        ExportBucket {
            metric: metric.into(),
            key: String::new(),
            start,
            min: mean,
            mean,
            max: mean,
            count,
        }
    }

    fn empty_export() -> Export {
        Export {
            version: EXPORT_VERSION,
            nas_id: "col-1".into(),
            until: T0,
            samples: Vec::new(),
            hourly: Vec::new(),
            daily: Vec::new(),
            backup_runs: Vec::new(),
            events: Vec::new(),
        }
    }

    #[test]
    fn import_replaces_buckets_only_with_more_samples() {
        let now = T0 + 2 * DAY;
        let mut app = HistoryStore::in_memory().expect("store opens");
        let mut first = empty_export();
        first.daily = vec![bucket_row("cpu", T0, 10.0, 5)];
        first.hourly = vec![bucket_row("cpu", T0 + DAY, 10.0, 5)];
        app.import(&first, "app-1", now).expect("first import");

        let read = |app: &HistoryStore, resolution: Resolution, start: i64| {
            app.points("app-1", Metric::Cpu, None, resolution, start, start)
                .expect("points")[0]
                .clone()
        };
        // Fewer samples: the stored values stay.
        let mut fewer = empty_export();
        fewer.daily = vec![bucket_row("cpu", T0, 99.0, 3)];
        fewer.hourly = vec![bucket_row("cpu", T0 + DAY, 99.0, 5)];
        let count = app.import(&fewer, "app-1", now).expect("import");
        assert_eq!((count.daily, count.hourly), (0, 0));
        assert_eq!(read(&app, Resolution::Daily, T0).mean, 10.0);
        assert_eq!(read(&app, Resolution::Hourly, T0 + DAY).mean, 10.0);

        // More samples: replaced.
        let mut more = empty_export();
        more.daily = vec![bucket_row("cpu", T0, 20.0, 6)];
        more.hourly = vec![bucket_row("cpu", T0 + DAY, 30.0, 12)];
        let count = app.import(&more, "app-1", now).expect("import");
        assert_eq!((count.daily, count.hourly), (1, 1));
        assert_eq!(read(&app, Resolution::Daily, T0).mean, 20.0);
        let hour = read(&app, Resolution::Hourly, T0 + DAY);
        assert_eq!((hour.mean, hour.count), (30.0, 12));
    }

    #[test]
    fn import_skips_unknown_rows_and_refuses_another_version() {
        let now = T0 + DAY;
        let mut app = HistoryStore::in_memory().expect("store opens");
        let mut export = empty_export();
        export.samples = vec![
            ExportSample {
                metric: "later.metric".into(),
                key: String::new(),
                at: T0,
                value: 1.0,
            },
            ExportSample {
                metric: "cpu".into(),
                key: String::new(),
                at: T0,
                value: 12.0,
            },
            ExportSample {
                metric: "disk.temp".into(),
                key: "disk-1".into(),
                at: T0,
                value: 35.0,
            },
        ];
        export.daily = vec![bucket_row("later.metric", T0, 1.0, 1)];
        export.backup_runs = vec![
            ExportBackupRun {
                task_id: "b1".into(),
                started_at: T0,
                finished_at: Some(T0 + 60),
                result: "ok".into(),
                message: None,
            },
            ExportBackupRun {
                task_id: "b2".into(),
                started_at: T0,
                finished_at: None,
                result: "later".into(),
                message: None,
            },
        ];
        let count = app.import(&export, "app-1", now).expect("import");
        assert_eq!(
            count,
            ImportCount {
                samples: 2,
                hourly: 0,
                daily: 0,
                backup_runs: 1,
                events: 0,
            }
        );
        let cpu = app
            .history("app-1", Metric::Cpu, Range::Day, now)
            .expect("cpu");
        assert_eq!(cpu[0].key, None);
        let disk = app
            .history("app-1", Metric::DiskTemp, Range::Day, now)
            .expect("disk");
        assert_eq!(disk[0].key.as_deref(), Some("disk-1"));

        let before = counts(&app);
        export.version = 2;
        assert!(matches!(
            app.import(&export, "app-1", now),
            Err(HistoryError::Format(_))
        ));
        assert_eq!(counts(&app), before);
    }

    #[test]
    fn last_seen_is_the_newest_snapshot() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        assert_eq!(store.last_seen("nas-1").expect("last seen"), None);
        for at in [T0 + HOUR, T0] {
            store
                .write_snapshot(&snapshot("nas-1", at, 0), at)
                .expect("snapshot is written");
        }
        assert_eq!(
            store.last_seen("nas-1").expect("last seen"),
            Some(T0 + HOUR)
        );
        assert_eq!(
            store.name_and_model("nas-1").expect("name"),
            Some((Some("Testgerät".into()), Some("TS-000".into())))
        );
        assert_eq!(store.name_and_model("nas-2").expect("name"), None);
    }

    #[test]
    fn stored_names_match_serde() {
        for provider in [
            ProviderKind::Qnap,
            ProviderKind::Synology,
            ProviderKind::TrueNas,
            ProviderKind::Unraid,
        ] {
            let json = serde_json::to_string(&provider).expect("provider serializes");
            assert_eq!(json, format!("\"{}\"", provider_str(provider)));
        }
        for result in [
            BackupResult::Ok,
            BackupResult::Error,
            BackupResult::Running,
            BackupResult::Never,
        ] {
            let json = serde_json::to_string(&result).expect("result serializes");
            assert_eq!(json, format!("\"{}\"", result_str(result)));
            assert_eq!(result_from(result_str(result)).expect("known"), result);
        }
        for level in [Health::Ok, Health::Warning, Health::Critical] {
            let json = serde_json::to_string(&level).expect("level serializes");
            assert_eq!(json, format!("\"{}\"", health_str(level)));
            assert_eq!(health_from(health_str(level)).expect("known"), level);
        }
        assert!(matches!(result_from("later"), Err(HistoryError::Value(_))));
        for metric in METRICS {
            assert_eq!(metric_from(metric.as_str()), Some(metric));
        }
        assert_eq!(metric_from("later"), None);
    }

    /// Every day of `source` and its last 24 hours, through JSON, into
    /// `target`; the counts of all imports added up.
    fn transfer(
        source: &HistoryStore,
        target: &mut HistoryStore,
        nas_id: &str,
        now: i64,
    ) -> ImportCount {
        let mut exports: Vec<Export> = source
            .days(nas_id)
            .expect("days")
            .into_iter()
            .filter_map(|day| source.export_day(nas_id, day, now).expect("day"))
            .collect();
        exports.extend(source.export_recent(nas_id, now).expect("recent"));
        let mut total = ImportCount::default();
        for export in exports {
            let json = serde_json::to_string(&export).expect("export serializes");
            let back: Export = serde_json::from_str(&json).expect("export parses");
            let count = target.import(&back, nas_id, now).expect("import");
            total.samples += count.samples;
            total.hourly += count.hourly;
            total.daily += count.daily;
            total.backup_runs += count.backup_runs;
            total.events += count.events;
        }
        total
    }

    #[test]
    fn days_and_the_last_day_give_the_mac_history_on_the_phone() {
        let end = T0 + 40 * DAY;
        let mut mac = collector("nas-1", end);
        mac.write_events(&[
            event(
                "nas-1",
                Some("disk-1"),
                T0 + 3 * DAY,
                Some(T0 + 3 * DAY + HOUR),
            ),
            event("nas-1", None, T0 + 10 * DAY, None),
        ])
        .expect("events");
        let days = mac.days("nas-1").expect("days");
        assert_eq!(days.len(), 41);
        assert_eq!(days[0], T0);
        assert!(days.iter().all(|day| day % DAY == 0));
        assert_eq!(mac.export_day("nas-1", T0 - DAY, end).expect("day"), None);
        assert_eq!(mac.export_day("nas-2", T0, end).expect("day"), None);
        assert_eq!(mac.export_recent("nas-2", end).expect("recent"), None);
        // A day holds its hours, no raw samples; the open event shows in
        // every day after its start.
        let day = mac
            .export_day("nas-1", T0 + 20 * DAY + 5, end)
            .expect("day")
            .expect("values");
        assert!(day.samples.is_empty());
        assert!(
            day.hourly
                .iter()
                .all(|h| h.start >= T0 + 20 * DAY && h.start < T0 + 21 * DAY)
        );
        assert!(day.daily.iter().all(|d| d.start == T0 + 20 * DAY));
        assert_eq!(day.events.len(), 1);
        assert_eq!(day.events[0].ended_at, None);
        let recent = mac
            .export_recent("nas-1", end)
            .expect("recent")
            .expect("samples");
        assert!(recent.samples.iter().all(|s| s.at >= end - DAY));

        let mut phone = HistoryStore::in_memory().expect("store opens");
        let taken = transfer(&mac, &mut phone, "nas-1", end);
        assert!(taken.hourly > 0 && taken.daily > 0 && taken.samples > 0);
        assert_eq!(taken.events, 2);
        for metric in ALL_METRICS {
            for range in [Range::Day, Range::Week, Range::Month, Range::Year] {
                let mine = phone.history("nas-1", metric, range, end).expect("phone");
                let theirs = mac.history("nas-1", metric, range, end).expect("mac");
                assert!(!mine.is_empty(), "{metric:?} {range:?}");
                assert_eq!(mine, theirs, "{metric:?} {range:?}");
            }
        }
        assert_eq!(
            phone.events(Some("nas-1"), T0, end).expect("events"),
            mac.events(Some("nas-1"), T0, end).expect("events")
        );
        assert_eq!(
            phone.backup_runs("nas-1", T0, end).expect("runs"),
            mac.backup_runs("nas-1", T0, end).expect("runs")
        );
        // The phone knows the NAS from its own list, not from the import.
        assert_eq!(count(&phone, "SELECT COUNT(*) FROM nas"), 0);

        // The next transfer of the same days changes nothing.
        let before = counts(&phone);
        assert_eq!(
            transfer(&mac, &mut phone, "nas-1", end),
            ImportCount::default()
        );
        assert_eq!(counts(&phone), before);
    }

    fn cpu_hour(store: &HistoryStore, start: i64, now: i64) -> Point {
        store
            .points("nas-1", Metric::Cpu, None, Resolution::Hourly, start, now)
            .expect("points")
            .into_iter()
            .find(|point| point.at == start)
            .expect("hour")
    }

    fn cpu(at: i64, value: f64) -> Sample {
        Sample {
            nas_id: "nas-1".into(),
            metric: Metric::Cpu,
            key: None,
            at,
            value,
        }
    }

    #[test]
    fn the_hour_with_more_samples_wins_and_is_never_mixed() {
        let hour = T0 + 5 * HOUR;
        let now = T0 + DAY;
        // The phone saw the hour once, the Mac 12 times.
        let mut phone = HistoryStore::in_memory().expect("store opens");
        phone
            .write_samples(&[cpu(hour + 60, 90.0)], now)
            .expect("own");
        let mut mac = empty_export();
        mac.hourly = vec![bucket_row("cpu", hour, 10.0, 12)];
        mac.daily = vec![bucket_row("cpu", T0, 10.0, 12)];
        let count = phone.import(&mac, "nas-1", now).expect("import");
        assert_eq!((count.hourly, count.daily), (1, 1));
        assert_eq!(cpu_hour(&phone, hour, now).count, 12);
        assert_eq!(cpu_hour(&phone, hour, now).mean, 10.0);

        // A later own sample in the hour rolls up 2 samples: the 12 of the
        // Mac stay, with neither own value mixed in, and so does the day.
        phone
            .write_samples(&[cpu(hour + 120, 95.0)], now)
            .expect("own");
        let kept = cpu_hour(&phone, hour, now);
        assert_eq!((kept.count, kept.mean), (12, 10.0));
        let day = phone
            .points("nas-1", Metric::Cpu, None, Resolution::Daily, T0, now)
            .expect("day");
        assert_eq!((day[0].count, day[0].mean), (12, 10.0));

        // The same import again counts nothing and changes nothing.
        let again = phone.import(&mac, "nas-1", now).expect("again");
        assert_eq!(again, ImportCount::default());

        // The other way: the phone has more samples than the Mac sends.
        let mut phone = HistoryStore::in_memory().expect("store opens");
        let own: Vec<Sample> = (0..12).map(|i| cpu(hour + i * 300, 50.0)).collect();
        phone.write_samples(&own, now).expect("own");
        let mut mac = empty_export();
        mac.hourly = vec![bucket_row("cpu", hour, 10.0, 5)];
        mac.daily = vec![bucket_row("cpu", T0, 10.0, 5)];
        let count = phone.import(&mac, "nas-1", now).expect("import");
        assert_eq!((count.hourly, count.daily), (0, 0));
        let kept = cpu_hour(&phone, hour, now);
        assert_eq!((kept.count, kept.mean), (12, 50.0));
    }

    #[test]
    fn an_imported_event_keeps_a_known_end_and_unknown_ones_are_skipped() {
        let now = T0 + DAY;
        let mut phone = HistoryStore::in_memory().expect("store opens");
        phone
            .write_events(&[event("nas-1", Some("disk-1"), T0, Some(T0 + HOUR))])
            .expect("own");
        let row = |kind: &str, level: &str, ended_at: Option<i64>| ExportEvent {
            kind: kind.into(),
            subject: "disk-1".into(),
            started_at: T0,
            ended_at,
            level: level.into(),
            text: "Platte 1 bei 55 °C".into(),
        };
        let mut export = empty_export();
        export.events = vec![
            // Still open at the Mac: the end of the phone stays.
            row("disk_temperature", "critical", None),
            row("later", "warning", None),
            row("disk_temperature", "bad", None),
        ];
        let count = phone.import(&export, "nas-1", now).expect("import");
        assert_eq!(count.events, 1);
        let events = phone.events(Some("nas-1"), T0, now).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ended_at, Some(T0 + HOUR));
        assert_eq!(events[0].level, Health::Critical);
        assert_eq!(events[0].text, "Platte 1 bei 55 °C");
    }

    #[test]
    fn an_export_without_events_reads_and_writes_as_before() {
        let json = r#"{"version":1,"nas_id":"col-1","until":1,"samples":[],"hourly":[],"daily":[],"backup_runs":[]}"#;
        let export: Export = serde_json::from_str(json).expect("collector export parses");
        assert!(export.events.is_empty());
        assert_eq!(serde_json::to_string(&export).expect("serializes"), json);
    }

    #[test]
    fn the_last_day_carries_the_backup_runs_of_a_week() {
        let end = T0 + 10 * DAY;
        let mut mac = collector("nas-1", end);
        // A run that began on an earlier day and finished later.
        mac.write_backup_runs(
            "nas-1",
            &[BackupRun {
                task_id: "b1".into(),
                started_at: end - 3 * DAY,
                finished_at: Some(end - 2 * DAY),
                result: BackupResult::Error,
                message: Some("Ziel voll".into()),
            }],
        )
        .expect("run");
        let recent = mac
            .export_recent("nas-1", end)
            .expect("recent")
            .expect("values");
        assert!(
            recent
                .backup_runs
                .iter()
                .all(|run| run.started_at >= end - 7 * DAY)
        );
        assert_eq!(recent.backup_runs.len(), 8);
        let mut phone = HistoryStore::in_memory().expect("store opens");
        phone.import(&recent, "nas-1", end).expect("import");
        let runs = phone
            .backup_runs("nas-1", end - 3 * DAY, end - 3 * DAY)
            .expect("runs");
        assert_eq!(runs[0].result, BackupResult::Error);
        assert_eq!(runs[0].finished_at, Some(end - 2 * DAY));

        // Only runs, no samples: still an export.
        let mut store = HistoryStore::in_memory().expect("store opens");
        store
            .write_backup_runs(
                "nas-1",
                &mac.backup_runs("nas-1", end - DAY, end).expect("runs"),
            )
            .expect("runs");
        assert!(store.export_recent("nas-1", end).expect("recent").is_some());
    }

    #[test]
    fn a_day_is_rolled_up_again_from_hours_of_both_devices() {
        let now = T0 + DAY;
        let mut phone = HistoryStore::in_memory().expect("store opens");
        // The Mac slept in hour 3, the phone measured it 4 times.
        let own: Vec<Sample> = (0..4).map(|i| cpu(T0 + 3 * HOUR + i * 600, 50.0)).collect();
        phone.write_samples(&own, now).expect("own");
        let mut mac = empty_export();
        mac.hourly = vec![
            bucket_row("cpu", T0 + HOUR, 10.0, 12),
            bucket_row("cpu", T0 + 2 * HOUR, 10.0, 12),
        ];
        mac.daily = vec![bucket_row("cpu", T0, 10.0, 24)];
        phone.import(&mac, "nas-1", now).expect("import");
        let day = phone
            .points("nas-1", Metric::Cpu, None, Resolution::Daily, T0, now)
            .expect("day");
        assert_eq!(day[0].count, 28);
        assert_eq!(day[0].max, 50.0);
        assert_eq!(day[0].mean, (24.0 * 10.0 + 4.0 * 50.0) / 28.0);
    }

    #[test]
    fn the_same_problem_seen_on_both_devices_is_one_event() {
        let now = T0 + DAY;
        let mut phone = HistoryStore::in_memory().expect("store opens");
        phone
            .write_events(&[event("nas-1", Some("disk-1"), T0 + 60, None)])
            .expect("own");
        let row = |ended_at: Option<i64>| ExportEvent {
            kind: "disk_temperature".into(),
            subject: "disk-1".into(),
            started_at: T0,
            ended_at,
            level: "warning".into(),
            text: "Platte 1 bei 50 °C".into(),
        };
        let mut export = empty_export();
        export.events = vec![row(None)];
        phone.import(&export, "nas-1", now).expect("open");
        let events = phone.events(Some("nas-1"), T0, now).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].started_at, T0 + 60);

        // Its end at the Mac ends the own one, also when it comes again.
        export.events = vec![row(Some(T0 + HOUR))];
        assert_eq!(
            phone.import(&export, "nas-1", now).expect("ended").events,
            1
        );
        phone.import(&export, "nas-1", now).expect("again");
        let events = phone.events(Some("nas-1"), T0, now).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ended_at, Some(T0 + HOUR));

        // A later problem of the same kind is an event of its own.
        export.events = vec![ExportEvent {
            started_at: T0 + 5 * HOUR,
            ..row(None)
        }];
        phone.import(&export, "nas-1", now).expect("later");
        assert_eq!(
            phone.events(Some("nas-1"), T0, now).expect("events").len(),
            2
        );
    }

    #[test]
    fn a_fresh_snapshot_elsewhere_ends_unreachable() {
        let mut store = HistoryStore::in_memory().expect("store opens");
        let unreachable = |started_at: i64| EventRecord {
            kind: "unreachable".into(),
            subject: None,
            ..event("nas-1", None, started_at, None)
        };
        store
            .write_events(&[unreachable(T0), event("nas-1", Some("disk-1"), T0, None)])
            .expect("events");
        assert!(!store.close_unreachable("nas-1", T0 - 1).expect("too early"));
        assert!(store.close_unreachable("nas-1", T0 + 900).expect("close"));
        let open = store.open_events("nas-1").expect("open");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].kind, "disk_temperature");
    }
}
