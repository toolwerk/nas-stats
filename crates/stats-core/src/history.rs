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

use nas_model::{BackupResult, BackupRun, Metric, ProviderKind, Sample, Snapshot};
use rusqlite::{Connection, Transaction, params};

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
    id INTEGER PRIMARY KEY,
    nas_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    key TEXT NOT NULL DEFAULT '',
    level TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_nas_at ON events (nas_id, at);
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
    /// delivered samples only.
    pub fn write_samples(&mut self, samples: &[Sample], now: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        write_in(&tx, samples, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Stores backup runs. A known run gets the new result; end and message
    /// stay when the new run does not know them.
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

fn upsert_runs(tx: &Transaction, nas_id: &str, runs: &[BackupRun]) -> Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO backup_runs (nas_id, task_id, started_at, finished_at, result, message)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (nas_id, task_id, started_at) DO UPDATE SET
             result = excluded.result,
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
fn write_in(tx: &Transaction, samples: &[Sample], now: i64) -> Result<()> {
    let mut hours = BTreeSet::new();
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO samples (nas_id, metric, key, at, value)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for sample in samples.iter().filter(|s| s.value.is_finite()) {
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
            "INSERT OR REPLACE INTO samples_hourly
                 (nas_id, metric, key, start, min, mean, max, count)
             SELECT nas_id, metric, key, ?4, MIN(value), AVG(value), MAX(value), COUNT(*)
             FROM samples
             WHERE nas_id = ?1 AND metric = ?2 AND key = ?3 AND at >= ?4 AND at < ?4 + ?5
             GROUP BY nas_id, metric, key",
        )?;
        for &(nas_id, metric, key, start) in &hours {
            hourly.execute(params![nas_id, metric, key, start, HOUR])?;
            days.insert((nas_id, metric, key, bucket(start, DAY)));
        }
    }

    {
        let mut daily = tx.prepare_cached(
            "INSERT OR REPLACE INTO samples_daily
                 (nas_id, metric, key, start, min, mean, max, count)
             SELECT nas_id, metric, key, ?4, MIN(min), SUM(mean * count) / SUM(count),
                    MAX(max), SUM(count)
             FROM samples_hourly
             WHERE nas_id = ?1 AND metric = ?2 AND key = ?3 AND start >= ?4 AND start < ?4 + ?5
             GROUP BY nas_id, metric, key",
        )?;
        for &(nas_id, metric, key, start) in &days {
            daily.execute(params![nas_id, metric, key, start, DAY])?;
        }
    }

    tx.prepare_cached("DELETE FROM samples WHERE at < ?1")?
        .execute(params![now - RAW_KEEP])?;
    tx.prepare_cached("DELETE FROM samples_hourly WHERE start < ?1")?
        .execute(params![now - HOURLY_KEEP])?;
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
        snap.backups[0].last_result = Some(BackupResult::Ok);
        store
            .write_snapshot(&snap, T0)
            .expect("snapshot is written");
        let runs = store
            .backup_runs("nas-1", T0 - DAY, T0)
            .expect("runs are read");
        assert_eq!(runs, vec![done]);
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
            .conn
            .execute(
                "INSERT INTO events (nas_id, at, kind, level, text) VALUES ('nas-1', ?1, 'x', 'info', 't')",
                params![T0],
            )
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
        assert!(matches!(result_from("later"), Err(HistoryError::Value(_))));
    }
}
