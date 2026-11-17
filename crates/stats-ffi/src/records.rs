//! Records the apps see. Each mirrors a type of `nas-model` with the same
//! field names, so `nas-model` stays free of FFI attributes; `From` maps the
//! model into the records. Units as in `nas-model`: times in Unix seconds
//! UTC, sizes in bytes, temperatures in degrees Celsius.

use nas_model as model;

/// Vendor family of a NAS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ProviderKind {
    Qnap,
    Synology,
    TrueNas,
    Unraid,
}

impl From<model::ProviderKind> for ProviderKind {
    fn from(kind: model::ProviderKind) -> Self {
        match kind {
            model::ProviderKind::Qnap => Self::Qnap,
            model::ProviderKind::Synology => Self::Synology,
            model::ProviderKind::TrueNas => Self::TrueNas,
            model::ProviderKind::Unraid => Self::Unraid,
        }
    }
}

impl From<ProviderKind> for model::ProviderKind {
    fn from(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Qnap => Self::Qnap,
            ProviderKind::Synology => Self::Synology,
            ProviderKind::TrueNas => Self::TrueNas,
            ProviderKind::Unraid => Self::Unraid,
        }
    }
}

/// Health reduced to three levels; `None` in an optional field is "unknown".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Health {
    Ok,
    Warning,
    Critical,
}

impl From<model::Health> for Health {
    fn from(health: model::Health) -> Self {
        match health {
            model::Health::Ok => Self::Ok,
            model::Health::Warning => Self::Warning,
            model::Health::Critical => Self::Critical,
        }
    }
}

/// A configured NAS as the NAS reports itself.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Nas {
    pub id: String,
    pub provider: ProviderKind,
    pub name: Option<String>,
    pub base_url: String,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub hostname: Option<String>,
}

impl From<model::Nas> for Nas {
    fn from(nas: model::Nas) -> Self {
        Self {
            id: nas.id,
            provider: nas.provider.into(),
            name: nas.name,
            base_url: nas.base_url,
            model: nas.model,
            firmware: nas.firmware,
            hostname: nas.hostname,
        }
    }
}

/// System values of a NAS.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SystemStatus {
    pub uptime_seconds: Option<u64>,
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub memory_total_bytes: Option<u64>,
    pub temperature_c: Option<f64>,
    pub cpu_temperature_c: Option<f64>,
    pub update_available: Option<bool>,
    pub update_version: Option<String>,
}

impl From<model::SystemStatus> for SystemStatus {
    fn from(s: model::SystemStatus) -> Self {
        Self {
            uptime_seconds: s.uptime_seconds,
            cpu_percent: s.cpu_percent,
            memory_percent: s.memory_percent,
            memory_total_bytes: s.memory_total_bytes,
            temperature_c: s.temperature_c,
            cpu_temperature_c: s.cpu_temperature_c,
            update_available: s.update_available,
            update_version: s.update_version,
        }
    }
}

/// Power state of a UPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum UpsState {
    Online,
    OnBattery,
}

impl From<model::UpsState> for UpsState {
    fn from(state: model::UpsState) -> Self {
        match state {
            model::UpsState::Online => Self::Online,
            model::UpsState::OnBattery => Self::OnBattery,
        }
    }
}

/// The UPS attached to a NAS.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Ups {
    pub state: Option<UpsState>,
    pub charge_percent: Option<f64>,
    pub runtime_seconds: Option<u64>,
}

impl From<model::Ups> for Ups {
    fn from(ups: model::Ups) -> Self {
        Self {
            state: ups.state.map(Into::into),
            charge_percent: ups.charge_percent,
            runtime_seconds: ups.runtime_seconds,
        }
    }
}

/// What a volume stands for at the vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum VolumeKind {
    Volume,
    Pool,
    Array,
    Share,
}

impl From<model::VolumeKind> for VolumeKind {
    fn from(kind: model::VolumeKind) -> Self {
        match kind {
            model::VolumeKind::Volume => Self::Volume,
            model::VolumeKind::Pool => Self::Pool,
            model::VolumeKind::Array => Self::Array,
            model::VolumeKind::Share => Self::Share,
        }
    }
}

/// State of a scrub, parity check or RAID scrubbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ScrubState {
    Running,
    Paused,
    Finished,
    Canceled,
    Failed,
}

impl From<model::ScrubState> for ScrubState {
    fn from(state: model::ScrubState) -> Self {
        match state {
            model::ScrubState::Running => Self::Running,
            model::ScrubState::Paused => Self::Paused,
            model::ScrubState::Finished => Self::Finished,
            model::ScrubState::Canceled => Self::Canceled,
            model::ScrubState::Failed => Self::Failed,
        }
    }
}

/// Last or current scrub of a volume.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScrubStatus {
    pub state: Option<ScrubState>,
    pub last_finished_at: Option<i64>,
    pub errors: Option<u64>,
    pub progress_percent: Option<f64>,
}

impl From<model::ScrubStatus> for ScrubStatus {
    fn from(s: model::ScrubStatus) -> Self {
        Self {
            state: s.state.map(Into::into),
            last_finished_at: s.last_finished_at,
            errors: s.errors,
            progress_percent: s.progress_percent,
        }
    }
}

/// A storage volume, pool, array or share.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub kind: VolumeKind,
    pub pool_id: Option<String>,
    pub used_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    /// Used share in percent, `None` when used or total space is unknown or
    /// the total is zero.
    pub used_percent: Option<f64>,
    pub health: Option<Health>,
    pub status_text: Option<String>,
    pub raid: Option<String>,
    pub scrub: Option<ScrubStatus>,
}

impl From<model::Volume> for Volume {
    fn from(v: model::Volume) -> Self {
        let used_percent = v.used_percent();
        Self {
            id: v.id,
            name: v.name,
            kind: v.kind.into(),
            pool_id: v.pool_id,
            used_bytes: v.used_bytes,
            total_bytes: v.total_bytes,
            used_percent,
            health: v.health.map(Into::into),
            status_text: v.status_text,
            raid: v.raid,
            scrub: v.scrub.map(Into::into),
        }
    }
}

/// A physical disk.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Disk {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    /// Hashed serial number; the plain serial never leaves the core.
    pub serial_hash: Option<String>,
    pub temperature_c: Option<f64>,
    pub health: Option<Health>,
    pub smart_text: Option<String>,
    pub power_on_hours: Option<u64>,
    pub spinning: Option<bool>,
}

impl From<model::Disk> for Disk {
    fn from(d: model::Disk) -> Self {
        Self {
            id: d.id,
            name: d.name,
            model: d.model,
            serial_hash: d.serial_hash,
            temperature_c: d.temperature_c,
            health: d.health.map(Into::into),
            smart_text: d.smart_text,
            power_on_hours: d.power_on_hours,
            spinning: d.spinning,
        }
    }
}

/// A fan of the NAS.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Fan {
    pub name: String,
    pub rpm: Option<u32>,
    pub health: Option<Health>,
}

impl From<model::Fan> for Fan {
    fn from(f: model::Fan) -> Self {
        Self {
            name: f.name,
            rpm: f.rpm,
            health: f.health.map(Into::into),
        }
    }
}

/// Kind of a backup task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum BackupKind {
    HyperBackup,
    ActiveBackup,
    Replication,
    CloudSync,
    Snapshot,
    Parity,
    Hbs3,
}

impl From<model::BackupKind> for BackupKind {
    fn from(kind: model::BackupKind) -> Self {
        match kind {
            model::BackupKind::HyperBackup => Self::HyperBackup,
            model::BackupKind::ActiveBackup => Self::ActiveBackup,
            model::BackupKind::Replication => Self::Replication,
            model::BackupKind::CloudSync => Self::CloudSync,
            model::BackupKind::Snapshot => Self::Snapshot,
            model::BackupKind::Parity => Self::Parity,
            model::BackupKind::Hbs3 => Self::Hbs3,
        }
    }
}

/// Result of a backup task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum BackupResult {
    Ok,
    Error,
    Running,
    Never,
}

impl From<model::BackupResult> for BackupResult {
    fn from(result: model::BackupResult) -> Self {
        match result {
            model::BackupResult::Ok => Self::Ok,
            model::BackupResult::Error => Self::Error,
            model::BackupResult::Running => Self::Running,
            model::BackupResult::Never => Self::Never,
        }
    }
}

/// A configured backup task with its last state.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BackupTask {
    pub id: String,
    pub name: String,
    pub kind: BackupKind,
    pub last_run: Option<i64>,
    pub last_result: Option<BackupResult>,
    pub next_run: Option<i64>,
}

impl From<model::BackupTask> for BackupTask {
    fn from(b: model::BackupTask) -> Self {
        Self {
            id: b.id,
            name: b.name,
            kind: b.kind.into(),
            last_run: b.last_run,
            last_result: b.last_result.map(Into::into),
            next_run: b.next_run,
        }
    }
}

/// A container or app running on the NAS.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Container {
    pub name: String,
    pub running: bool,
}

impl From<model::Container> for Container {
    fn from(c: model::Container) -> Self {
        Self {
            name: c.name,
            running: c.running,
        }
    }
}

/// Severity of an alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

impl From<model::AlertLevel> for AlertLevel {
    fn from(level: model::AlertLevel) -> Self {
        match level {
            model::AlertLevel::Info => Self::Info,
            model::AlertLevel::Warning => Self::Warning,
            model::AlertLevel::Critical => Self::Critical,
        }
    }
}

/// An alert the NAS reports.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Alert {
    pub level: AlertLevel,
    pub text: String,
    pub since: Option<i64>,
}

impl From<model::Alert> for Alert {
    fn from(a: model::Alert) -> Self {
        Self {
            level: a.level.into(),
            text: a.text,
            since: a.since,
        }
    }
}

/// Throughput of a network interface.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NetInterface {
    pub name: String,
    pub rx_bytes_per_sec: Option<f64>,
    pub tx_bytes_per_sec: Option<f64>,
}

impl From<model::NetInterface> for NetInterface {
    fn from(n: model::NetInterface) -> Self {
        Self {
            name: n.name,
            rx_bytes_per_sec: n.rx_bytes_per_sec,
            tx_bytes_per_sec: n.tx_bytes_per_sec,
        }
    }
}

/// The rule a finding comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum FindingKind {
    Alert,
    DiskHealth,
    DiskTemperature,
    VolumeHealth,
    VolumeFill,
    FanHealth,
    UpsOnBattery,
    BackupFailed,
    BackupOverdue,
}

impl From<model::FindingKind> for FindingKind {
    fn from(kind: model::FindingKind) -> Self {
        match kind {
            model::FindingKind::Alert => Self::Alert,
            model::FindingKind::DiskHealth => Self::DiskHealth,
            model::FindingKind::DiskTemperature => Self::DiskTemperature,
            model::FindingKind::VolumeHealth => Self::VolumeHealth,
            model::FindingKind::VolumeFill => Self::VolumeFill,
            model::FindingKind::FanHealth => Self::FanHealth,
            model::FindingKind::UpsOnBattery => Self::UpsOnBattery,
            model::FindingKind::BackupFailed => Self::BackupFailed,
            model::FindingKind::BackupOverdue => Self::BackupOverdue,
        }
    }
}

/// Something about a NAS that needs attention (see
/// `nas_model::Snapshot::findings`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Finding {
    pub kind: FindingKind,
    /// `Warning` or `Critical`.
    pub level: Health,
    /// Id of the disk, volume or backup task, name of a fan.
    pub subject: Option<String>,
    /// German text, e.g. „Platte 3 bei 52 °C“.
    pub text: String,
}

impl From<model::Finding> for Finding {
    fn from(f: model::Finding) -> Self {
        Self {
            kind: f.kind.into(),
            level: f.level.into(),
            subject: f.subject,
            text: f.text,
        }
    }
}

/// Limits from which a value becomes a [`Finding`], as the user set them.
/// Taken over unchecked; the app checks them before it hands them over.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct Thresholds {
    /// Disk temperature in degrees Celsius from which a disk is a warning.
    pub disk_temp_warning_c: f64,
    /// Fill level in percent from which a volume is a warning.
    pub fill_warning_percent: f64,
    /// Fill level in percent from which a volume is critical.
    pub fill_critical_percent: f64,
}

impl From<model::Thresholds> for Thresholds {
    fn from(t: model::Thresholds) -> Self {
        Self {
            disk_temp_warning_c: t.disk_temp_warning_c,
            fill_warning_percent: t.fill_warning_percent,
            fill_critical_percent: t.fill_critical_percent,
        }
    }
}

impl From<Thresholds> for model::Thresholds {
    /// The backup limit is not set in the app and stays at its standard of
    /// two days.
    fn from(t: Thresholds) -> Self {
        Self {
            disk_temp_warning_c: t.disk_temp_warning_c,
            fill_warning_percent: t.fill_warning_percent,
            fill_critical_percent: t.fill_critical_percent,
            ..Self::default()
        }
    }
}

/// State of one NAS from a single run over all endpoints.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Snapshot {
    pub nas: Nas,
    pub observed_at: i64,
    pub system: SystemStatus,
    pub ups: Option<Ups>,
    pub volumes: Vec<Volume>,
    pub disks: Vec<Disk>,
    pub fans: Vec<Fan>,
    pub backups: Vec<BackupTask>,
    pub containers: Vec<Container>,
    pub alerts: Vec<Alert>,
    pub interfaces: Vec<NetInterface>,
    /// Worst health the NAS reports itself (see
    /// `nas_model::Snapshot::worst_health`), `None` when nothing is known.
    pub worst_health: Option<Health>,
    /// What needs attention, critical first, checked against the thresholds
    /// the app set with `NasCore::set_thresholds` (standard: disk 50 °C,
    /// volume 80 and 90 %; backup always two days).
    pub findings: Vec<Finding>,
    /// Ids of the volumes whose fill level counts, in the order of
    /// `volumes` (see `nas_model::Snapshot::capacity_volumes`): a pool and
    /// the volume in it count once.
    pub capacity_volume_ids: Vec<String>,
    /// True for demo values, which the app must mark as such.
    pub demo: bool,
}

impl Snapshot {
    /// Record of a snapshot read from a NAS (`demo` false).
    pub(crate) fn real(snapshot: model::Snapshot, thresholds: &model::Thresholds) -> Self {
        Self::convert(snapshot, thresholds, false)
    }

    /// Record of demo values (`demo` true).
    pub(crate) fn demo(snapshot: model::Snapshot, thresholds: &model::Thresholds) -> Self {
        Self::convert(snapshot, thresholds, true)
    }

    fn convert(s: model::Snapshot, thresholds: &model::Thresholds, demo: bool) -> Self {
        let worst_health = s.worst_health().map(Into::into);
        let findings = s.findings(thresholds).into_iter().map(Into::into).collect();
        let capacity_volume_ids = s
            .capacity_volumes()
            .into_iter()
            .map(|v| v.id.clone())
            .collect();
        Self {
            nas: s.nas.into(),
            observed_at: s.observed_at,
            system: s.system.into(),
            ups: s.ups.map(Into::into),
            volumes: s.volumes.into_iter().map(Into::into).collect(),
            disks: s.disks.into_iter().map(Into::into).collect(),
            fans: s.fans.into_iter().map(Into::into).collect(),
            backups: s.backups.into_iter().map(Into::into).collect(),
            containers: s.containers.into_iter().map(Into::into).collect(),
            alerts: s.alerts.into_iter().map(Into::into).collect(),
            interfaces: s.interfaces.into_iter().map(Into::into).collect(),
            worst_health,
            findings,
            capacity_volume_ids,
            demo,
        }
    }
}

/// What the app stores about a NAS (besides the credentials, which live in
/// the keychain) and hands to the core at every start.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NasConfig {
    /// Local id assigned by the app.
    pub id: String,
    pub provider: ProviderKind,
    /// `https://host:port` of the NAS.
    pub base_url: String,
    /// SHA-256 fingerprint of the certificate the user confirmed, `AB:CD:…`.
    pub fingerprint: String,
}

/// One NAS of the collector on the NAS (#46), as it lists them.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CollectorNas {
    /// Id of the NAS at the collector, for
    /// `NasCore::import_from_collector`.
    pub id: String,
    pub provider: ProviderKind,
    /// Name and model as the NAS reported them last; `None` before.
    pub name: Option<String>,
    pub model: Option<String>,
    /// Address the collector queries, `https://…`.
    pub base_url: String,
    /// Fingerprint of the certificate of the NAS, `AB:CD:…`; matches the one
    /// of the same NAS in the app.
    pub fingerprint: String,
    /// Time of the newest snapshot at the collector, Unix seconds UTC.
    pub last_seen: Option<i64>,
    /// The collector stopped after a rejected login.
    pub blocked: bool,
    /// Error of the last query of the collector, as text.
    pub error: Option<String>,
}

impl From<stats_core::CollectorNas> for CollectorNas {
    fn from(nas: stats_core::CollectorNas) -> Self {
        Self {
            id: nas.id,
            provider: nas.provider.into(),
            name: nas.name,
            model: nas.model,
            base_url: nas.base_url,
            fingerprint: nas.fingerprint,
            last_seen: nas.last_seen,
            blocked: nas.blocked,
            error: nas.error,
        }
    }
}

/// What one import from the collector took over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CollectorImport {
    /// Time of the export, Unix seconds UTC: `since` of the next import.
    pub until: i64,
    /// Raw samples that were new or had another value.
    pub samples: u32,
    /// Hourly and daily values written or replaced.
    pub hourly: u32,
    pub daily: u32,
    /// Backup runs that were new or changed.
    pub backup_runs: u32,
}

/// Kind of the last error of a NAS, one per `ProviderError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ProblemKind {
    Unreachable,
    Timeout,
    LoginFailed,
    SecondFactorRequired,
    /// The NAS blocks the address of this device (DSM error 407).
    IpBlocked,
    PermissionDenied,
    CertificateChanged,
    Parse,
    Http,
}

/// Why the last query of a NAS failed.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NasProblem {
    pub kind: ProblemKind,
    /// Text of the error (German), for the log and as a fallback in the app.
    pub message: String,
    /// The core does not query this NAS again until the user changes the
    /// credentials or adds the NAS again.
    pub stops_polling: bool,
    /// The NAS gave no answer: no connection, time-out, or HTTP 502/503/504
    /// of a proxy. The app shows it as "not reachable".
    pub no_answer: bool,
    /// For `CertificateChanged`: the fingerprint trusted so far.
    pub old_fingerprint: Option<String>,
    /// For `CertificateChanged`: the fingerprint the NAS presents now.
    pub new_fingerprint: Option<String>,
    /// For `Http`: the status code.
    pub http_status: Option<u16>,
}

impl From<&model::ProviderError> for NasProblem {
    fn from(e: &model::ProviderError) -> Self {
        use model::ProviderError as E;
        let kind = match e {
            E::Unreachable(_) => ProblemKind::Unreachable,
            E::Timeout => ProblemKind::Timeout,
            E::LoginFailed => ProblemKind::LoginFailed,
            E::SecondFactorRequired => ProblemKind::SecondFactorRequired,
            E::IpBlocked => ProblemKind::IpBlocked,
            E::PermissionDenied => ProblemKind::PermissionDenied,
            E::CertificateChanged { .. } => ProblemKind::CertificateChanged,
            E::Parse(_) => ProblemKind::Parse,
            E::Http(_) => ProblemKind::Http,
        };
        let (old_fingerprint, new_fingerprint) = match e {
            E::CertificateChanged { old, new } => (Some(old.clone()), Some(new.clone())),
            _ => (None, None),
        };
        let http_status = match e {
            E::Http(status) => Some(*status),
            _ => None,
        };
        Self {
            kind,
            message: e.to_string(),
            stops_polling: e.stops_polling(),
            no_answer: e.is_no_answer(),
            old_fingerprint,
            new_fingerprint,
            http_status,
        }
    }
}

/// What the core knows about one NAS right now.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NasState {
    pub nas_id: String,
    /// Last successful snapshot; it stays after an error.
    pub snapshot: Option<Snapshot>,
    /// Error of the last query, `None` after a success or before the first.
    pub problem: Option<NasProblem>,
    /// Start of the last query, Unix seconds UTC.
    pub last_attempt_at: Option<i64>,
    /// The core does not query this NAS until the user acts.
    pub blocked: bool,
}

/// What a history series measures; the unit follows from the metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Metric {
    /// Used space of a volume, in bytes.
    VolumeUsed,
    /// Total space of a volume, in bytes.
    VolumeTotal,
    /// Disk temperature, in degrees Celsius.
    DiskTemp,
    /// CPU load, in percent.
    Cpu,
    /// Used memory, in percent.
    Mem,
    /// Received bytes per second of an interface.
    NetRx,
    /// Sent bytes per second of an interface.
    NetTx,
    /// System temperature, in degrees Celsius.
    SysTemp,
}

impl From<Metric> for model::Metric {
    fn from(metric: Metric) -> Self {
        match metric {
            Metric::VolumeUsed => Self::VolumeUsed,
            Metric::VolumeTotal => Self::VolumeTotal,
            Metric::DiskTemp => Self::DiskTemp,
            Metric::Cpu => Self::Cpu,
            Metric::Mem => Self::Mem,
            Metric::NetRx => Self::NetRx,
            Metric::NetTx => Self::NetTx,
            Metric::SysTemp => Self::SysTemp,
        }
    }
}

/// Time range of a chart, ending now. Named `HistoryRange` because `Range`
/// is taken in Swift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum HistoryRange {
    /// Last 24 hours, every sample.
    Day,
    /// Last 7 days, hourly values.
    Week,
    /// Last 30 days, hourly values.
    Month,
    /// Last 365 days, daily values.
    Year,
}

impl From<HistoryRange> for stats_core::Range {
    fn from(range: HistoryRange) -> Self {
        match range {
            HistoryRange::Day => Self::Day,
            HistoryRange::Week => Self::Week,
            HistoryRange::Month => Self::Month,
            HistoryRange::Year => Self::Year,
        }
    }
}

/// One point of a series: a sample, or an hour or day rolled up. For a
/// sample min, mean and max are the value.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistoryPoint {
    /// Time of the sample or start of the hour or day, Unix seconds UTC.
    pub at: i64,
    pub min: f64,
    pub mean: f64,
    pub max: f64,
    /// Number of samples in the point.
    pub count: u32,
}

impl From<stats_core::Point> for HistoryPoint {
    fn from(p: stats_core::Point) -> Self {
        Self {
            at: p.at,
            min: p.min,
            mean: p.mean,
            max: p.max,
            count: p.count,
        }
    }
}

/// All points of one metric and key, oldest first.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistorySeries {
    /// Volume, disk or interface id; `None` for values of the whole NAS.
    pub key: Option<String>,
    pub points: Vec<HistoryPoint>,
}

impl From<stats_core::Series> for HistorySeries {
    fn from(s: stats_core::Series) -> Self {
        Self {
            key: s.key,
            points: s.points.into_iter().map(Into::into).collect(),
        }
    }
}

/// One past run of a backup task.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BackupRun {
    /// Id of the backup task the run belongs to.
    pub task_id: String,
    /// Start, Unix seconds UTC.
    pub started_at: i64,
    /// End, Unix seconds UTC; `None` while it runs or when unknown.
    pub finished_at: Option<i64>,
    pub result: BackupResult,
    /// Message of the vendor about the run.
    pub message: Option<String>,
}

impl From<model::BackupRun> for BackupRun {
    fn from(r: model::BackupRun) -> Self {
        Self {
            task_id: r.task_id,
            started_at: r.started_at,
            finished_at: r.finished_at,
            result: r.result.into(),
            message: r.message,
        }
    }
}

/// Forecast "full in X days" of a volume.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Forecast {
    /// Days from now until the volume is full; 0 when it is full already.
    pub days_to_full: f64,
    /// Time the volume is full, Unix seconds UTC.
    pub full_at: i64,
    /// Growth of the used space in bytes per day.
    pub growth_per_day: f64,
}

impl From<stats_core::Forecast> for Forecast {
    fn from(f: stats_core::Forecast) -> Self {
        Self {
            days_to_full: f.days_to_full,
            full_at: f.full_at,
            growth_per_day: f.growth_per_day,
        }
    }
}

/// The rule an event comes from: every finding kind plus `Unreachable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum EventKind {
    Alert,
    DiskHealth,
    DiskTemperature,
    VolumeHealth,
    VolumeFill,
    FanHealth,
    UpsOnBattery,
    BackupFailed,
    BackupOverdue,
    /// No snapshot for 15 minutes or longer.
    Unreachable,
}

impl From<model::EventKind> for EventKind {
    fn from(kind: model::EventKind) -> Self {
        match kind {
            model::EventKind::Alert => Self::Alert,
            model::EventKind::DiskHealth => Self::DiskHealth,
            model::EventKind::DiskTemperature => Self::DiskTemperature,
            model::EventKind::VolumeHealth => Self::VolumeHealth,
            model::EventKind::VolumeFill => Self::VolumeFill,
            model::EventKind::FanHealth => Self::FanHealth,
            model::EventKind::UpsOnBattery => Self::UpsOnBattery,
            model::EventKind::BackupFailed => Self::BackupFailed,
            model::EventKind::BackupOverdue => Self::BackupOverdue,
            model::EventKind::Unreachable => Self::Unreachable,
        }
    }
}

/// Something about a NAS that needed attention, from its start to its end
/// (see `nas_model::Event`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NasEvent {
    pub nas_id: String,
    pub kind: EventKind,
    /// Id of the disk, volume or backup task; the name for a fan; the text
    /// of the alert for an alert; `None` for the UPS and unreachable.
    pub subject: Option<String>,
    /// Highest level while the event was open, `Warning` or `Critical`.
    pub level: Health,
    /// German text of the start or of the last raise, e.g. „Platte 3 bei 52 °C“.
    pub text: String,
    /// Start, Unix seconds UTC.
    pub started_at: i64,
    /// End, Unix seconds UTC; `None` while the event is open.
    pub ended_at: Option<i64>,
}

impl From<model::Event> for NasEvent {
    fn from(e: model::Event) -> Self {
        Self {
            nas_id: e.nas_id,
            kind: e.kind.into(),
            subject: e.subject,
            level: e.level.into(),
            text: e.text,
            started_at: e.started_at,
            ended_at: e.ended_at,
        }
    }
}

/// Receives the state of a NAS after every finished query of the planner,
/// also after a failed one (see `NasCore::start_planner`).
///
/// Called from a thread of the core, one call at a time, in the order the
/// queries finished. The app must return at once and never wait
/// synchronously for its main thread or call back into `NasCore` from the
/// call itself (only from a task it starts): the core holds a lock during
/// the call, so `add_nas` and the like would wait forever.
#[uniffi::export(with_foreign)]
pub trait StateListener: Send + Sync {
    fn state_changed(&self, state: NasState);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_change_carries_both_fingerprints() {
        let problem = NasProblem::from(&model::ProviderError::CertificateChanged {
            old: "AA".into(),
            new: "BB".into(),
        });
        assert_eq!(problem.kind, ProblemKind::CertificateChanged);
        assert!(problem.stops_polling);
        assert_eq!(problem.old_fingerprint.as_deref(), Some("AA"));
        assert_eq!(problem.new_fingerprint.as_deref(), Some("BB"));
        assert_eq!(problem.http_status, None);
        assert!(problem.message.contains("AA"));
    }

    #[test]
    fn http_problem_carries_the_status() {
        let problem = NasProblem::from(&model::ProviderError::Http(503));
        assert_eq!(problem.kind, ProblemKind::Http);
        assert!(!problem.stops_polling);
        assert!(problem.no_answer);
        assert_eq!(problem.http_status, Some(503));
        assert_eq!(problem.message, "HTTP-Status 503");
    }

    #[test]
    fn only_a_missing_answer_is_flagged_no_answer() {
        assert!(!NasProblem::from(&model::ProviderError::Http(500)).no_answer);
        assert!(
            !NasProblem::from(&model::ProviderError::CertificateChanged {
                old: "AA".into(),
                new: "BB".into(),
            })
            .no_answer
        );
        assert!(NasProblem::from(&model::ProviderError::Timeout).no_answer);
    }
}
