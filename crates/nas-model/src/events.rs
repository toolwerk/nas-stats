//! Events: when something about a NAS began to need attention and when it
//! was over.
//!
//! Events come from changes of state, not from the logs of the vendors. Each
//! new [`Snapshot`] is compared with the events still open (not with the
//! previous snapshot), so the hysteresis holds over many queries and over a
//! restart of the app. Storing the events is up to the caller.

use serde::{Deserialize, Serialize};

use crate::backup::BackupResult;
use crate::findings::{FindingKind, Thresholds};
use crate::health::Health;
use crate::nas::Nas;
use crate::snapshot::Snapshot;

/// Seconds without a snapshot after which a NAS counts as unreachable.
pub const UNREACHABLE_AFTER_SECS: i64 = 900;

/// Degrees Celsius below the warning threshold a disk must reach before its
/// temperature event is over.
pub const DISK_TEMP_RECOVERY_C: f64 = 3.0;

/// Percentage points below the warning threshold a volume must reach before
/// its fill event is over.
pub const FILL_RECOVERY_PERCENT: f64 = 1.0;

/// The rule an [`Event`] comes from: every [`FindingKind`] plus
/// [`EventKind::Unreachable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// An alert of the NAS with level warning or critical.
    Alert,
    /// A disk whose SMART state the NAS reports as not good.
    DiskHealth,
    /// A disk at or above the temperature threshold.
    DiskTemperature,
    /// A volume, pool, array or share the NAS reports as not good.
    VolumeHealth,
    /// A volume at or above a fill threshold.
    VolumeFill,
    /// A fan the NAS reports as not good.
    FanHealth,
    /// The UPS runs on battery.
    UpsOnBattery,
    /// The last run of a backup task failed.
    BackupFailed,
    /// The last run of a backup task is older than allowed.
    BackupOverdue,
    /// No snapshot for [`UNREACHABLE_AFTER_SECS`] or longer.
    Unreachable,
}

impl From<FindingKind> for EventKind {
    fn from(kind: FindingKind) -> Self {
        match kind {
            FindingKind::Alert => Self::Alert,
            FindingKind::DiskHealth => Self::DiskHealth,
            FindingKind::DiskTemperature => Self::DiskTemperature,
            FindingKind::VolumeHealth => Self::VolumeHealth,
            FindingKind::VolumeFill => Self::VolumeFill,
            FindingKind::FanHealth => Self::FanHealth,
            FindingKind::UpsOnBattery => Self::UpsOnBattery,
            FindingKind::BackupFailed => Self::BackupFailed,
            FindingKind::BackupOverdue => Self::BackupOverdue,
        }
    }
}

/// Something about a NAS that needed attention, from its start to its end.
///
/// An event is identified by `nas_id`, `kind` and `subject`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
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

impl Event {
    /// Whether the event is still open.
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none()
    }

    fn has_key(&self, kind: EventKind, subject: Option<&str>) -> bool {
        self.kind == kind && self.subject.as_deref() == subject
    }
}

/// What happened to an event, each with the event after the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventChange {
    /// A new event began.
    Opened(Event),
    /// An open event reached a higher level; level and text are new, the
    /// start stays.
    Raised(Event),
    /// An open event is over.
    Closed(Event),
}

impl EventChange {
    /// The event after the change.
    pub fn event(&self) -> &Event {
        match self {
            Self::Opened(e) | Self::Raised(e) | Self::Closed(e) => e,
        }
    }
}

/// Compares `snapshot` with the events in `open` and returns what changed.
///
/// Only open events of the NAS of the snapshot count; others are ignored.
/// A finding without an open event opens one; a finding above the level of
/// its open event raises it; a lower level changes nothing. An open event
/// without a finding closes, unless it is still active: a disk at most
/// [`DISK_TEMP_RECOVERY_C`] below the temperature threshold, a volume at
/// most [`FILL_RECOVERY_PERCENT`] below the fill threshold, or a part that
/// is still there but whose value is unknown. A backup task stays open while
/// a new run is running (or, when overdue, reports no run), so only a
/// successful run ends it. A part that is gone closes its
/// event; an open [`EventKind::Unreachable`] always closes, as the NAS
/// answered.
///
/// Order: opened and raised events in the order of the findings, then the
/// closed ones in the order of `open`.
pub fn detect_events(
    open: &[Event],
    snapshot: &Snapshot,
    thresholds: &Thresholds,
) -> Vec<EventChange> {
    let nas_id = snapshot.nas.id.as_str();
    let open: Vec<&Event> = open
        .iter()
        .filter(|e| e.is_open() && e.nas_id == nas_id)
        .collect();
    let at = snapshot.observed_at;
    let mut changes = Vec::new();
    let mut seen: Vec<(EventKind, Option<String>)> = Vec::new();

    for finding in snapshot.findings(thresholds) {
        let kind = EventKind::from(finding.kind);
        let subject = match finding.kind {
            FindingKind::Alert => Some(finding.text.clone()),
            _ => finding.subject,
        };
        if seen.iter().any(|(k, s)| *k == kind && *s == subject) {
            continue;
        }
        match open.iter().find(|e| e.has_key(kind, subject.as_deref())) {
            None => changes.push(EventChange::Opened(Event {
                nas_id: nas_id.to_owned(),
                kind,
                subject: subject.clone(),
                level: finding.level,
                text: finding.text,
                started_at: at,
                ended_at: None,
            })),
            Some(event) if finding.level > event.level => {
                changes.push(EventChange::Raised(Event {
                    level: finding.level,
                    text: finding.text,
                    ..(*event).clone()
                }));
            }
            Some(_) => {}
        }
        seen.push((kind, subject));
    }

    for event in open {
        let has_finding = seen.iter().any(|(k, s)| event.has_key(*k, s.as_deref()));
        if has_finding || still_active(event, snapshot, thresholds) {
            continue;
        }
        changes.push(EventChange::Closed(Event {
            ended_at: Some(at),
            ..event.clone()
        }));
    }
    changes
}

/// Whether an open event without a finding stays open: within the
/// hysteresis, or the part is there but its value is unknown.
fn still_active(event: &Event, snapshot: &Snapshot, thresholds: &Thresholds) -> bool {
    let subject = event.subject.as_deref();
    let disk = || {
        snapshot
            .disks
            .iter()
            .find(|d| Some(d.id.as_str()) == subject)
    };
    let backup = || {
        snapshot
            .backups
            .iter()
            .find(|b| Some(b.id.as_str()) == subject)
    };
    match event.kind {
        EventKind::DiskTemperature => disk().is_some_and(|d| {
            d.temperature_c
                .is_none_or(|t| t >= thresholds.disk_temp_warning_c - DISK_TEMP_RECOVERY_C)
        }),
        EventKind::VolumeFill => snapshot
            .capacity_volumes()
            .into_iter()
            .find(|v| Some(v.id.as_str()) == subject)
            .is_some_and(|v| {
                v.used_percent()
                    .is_none_or(|p| p >= thresholds.fill_warning_percent - FILL_RECOVERY_PERCENT)
            }),
        EventKind::DiskHealth => disk().is_some_and(|d| d.health.is_none()),
        EventKind::VolumeHealth => snapshot
            .volumes
            .iter()
            .find(|v| Some(v.id.as_str()) == subject)
            .is_some_and(|v| v.health.is_none()),
        EventKind::FanHealth => snapshot
            .fans
            .iter()
            .find(|f| Some(f.name.as_str()) == subject)
            .is_some_and(|f| f.health.is_none()),
        EventKind::UpsOnBattery => snapshot.ups.as_ref().is_some_and(|u| u.state.is_none()),
        EventKind::BackupFailed => {
            backup().is_some_and(|b| matches!(b.last_result, None | Some(BackupResult::Running)))
        }
        EventKind::BackupOverdue => backup().is_some_and(|b| {
            b.last_run.is_none()
                || matches!(
                    b.last_result,
                    Some(BackupResult::Running | BackupResult::Never)
                )
        }),
        EventKind::Alert | EventKind::Unreachable => false,
    }
}

/// Opens an [`EventKind::Unreachable`] event when the last snapshot of `nas`
/// is [`UNREACHABLE_AFTER_SECS`] or more before `now`.
///
/// The event starts at `last_seen_at`, is critical and reads
/// „{Name} nicht erreichbar“ (name, else hostname, else base URL). `None`
/// when the NAS was never reached (`last_seen_at` is `None`), when it is
/// not yet overdue, or when an unreachable event of it is already open.
/// [`detect_events`] closes the event with the next snapshot.
pub fn detect_unreachable(
    open: &[Event],
    nas: &Nas,
    last_seen_at: Option<i64>,
    now: i64,
) -> Option<EventChange> {
    let last_seen_at = last_seen_at?;
    if now.saturating_sub(last_seen_at) < UNREACHABLE_AFTER_SECS {
        return None;
    }
    if open
        .iter()
        .any(|e| e.is_open() && e.nas_id == nas.id && e.kind == EventKind::Unreachable)
    {
        return None;
    }
    let name = nas
        .name
        .as_deref()
        .or(nas.hostname.as_deref())
        .unwrap_or(&nas.base_url);
    Some(EventChange::Opened(Event {
        nas_id: nas.id.clone(),
        kind: EventKind::Unreachable,
        subject: None,
        level: Health::Critical,
        text: format!("{name} nicht erreichbar"),
        started_at: last_seen_at,
        ended_at: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{BackupKind, BackupResult, BackupTask};
    use crate::device::{Alert, AlertLevel, Fan, Ups, UpsState};
    use crate::nas::{ProviderKind, SystemStatus};
    use crate::storage::{Disk, Volume, VolumeKind};

    const T0: i64 = 1_700_000_000;
    const STEP: i64 = 300;

    /// Puts a state into a snapshot.
    type Change = fn(&mut Snapshot);

    fn empty(at: i64) -> Snapshot {
        Snapshot {
            nas: Nas {
                id: "nas-1".into(),
                provider: ProviderKind::Qnap,
                name: Some("Keller".into()),
                base_url: "https://nas.example:8080".into(),
                model: None,
                firmware: None,
                hostname: Some("keller-nas".into()),
            },
            observed_at: at,
            system: SystemStatus::default(),
            ups: None,
            volumes: Vec::new(),
            disks: Vec::new(),
            fans: Vec::new(),
            backups: Vec::new(),
            containers: Vec::new(),
            alerts: Vec::new(),
            interfaces: Vec::new(),
        }
    }

    fn disk(temperature_c: Option<f64>, health: Option<Health>) -> Disk {
        Disk {
            id: "disk-3".into(),
            name: "Platte 3".into(),
            model: None,
            serial_hash: None,
            temperature_c,
            health,
            smart_text: None,
            power_on_hours: None,
            spinning: None,
        }
    }

    /// DataVol1 at `per_mille` / 10 percent.
    fn volume(per_mille: Option<u64>, health: Option<Health>) -> Volume {
        Volume {
            id: "vol-1".into(),
            name: "DataVol1".into(),
            kind: VolumeKind::Volume,
            pool_id: None,
            used_bytes: per_mille,
            total_bytes: Some(1000),
            health,
            status_text: None,
            raid: None,
            scrub: None,
        }
    }

    fn fan(health: Option<Health>) -> Fan {
        Fan {
            name: "Lüfter 1".into(),
            rpm: None,
            health,
        }
    }

    fn ups(state: Option<UpsState>) -> Ups {
        Ups {
            state,
            charge_percent: None,
            runtime_seconds: None,
        }
    }

    fn backup(last_run: Option<i64>, last_result: Option<BackupResult>) -> BackupTask {
        BackupTask {
            id: "b1".into(),
            name: "Offsite".into(),
            kind: BackupKind::Hbs3,
            last_run,
            last_result,
            next_run: None,
        }
    }

    fn alert(text: &str) -> Alert {
        Alert {
            level: AlertLevel::Warning,
            text: text.into(),
            since: None,
        }
    }

    /// Runs `detect_events` and applies the changes to `open`, as a store
    /// would.
    fn step(open: &mut Vec<Event>, snapshot: &Snapshot) -> Vec<EventChange> {
        let changes = detect_events(open, snapshot, &Thresholds::default());
        apply(open, &changes);
        changes
    }

    fn apply(open: &mut Vec<Event>, changes: &[EventChange]) {
        for change in changes {
            match change {
                EventChange::Opened(e) => open.push(e.clone()),
                EventChange::Raised(e) | EventChange::Closed(e) => {
                    let slot = open
                        .iter_mut()
                        .find(|o| {
                            o.is_open()
                                && o.nas_id == e.nas_id
                                && o.has_key(e.kind, e.subject.as_deref())
                        })
                        .expect("changed event is open");
                    *slot = e.clone();
                }
            }
        }
    }

    fn opened(changes: &[EventChange]) -> Event {
        assert_eq!(changes.len(), 1, "{changes:?}");
        match &changes[0] {
            EventChange::Opened(e) => e.clone(),
            other => panic!("expected opened, got {other:?}"),
        }
    }

    fn closed(changes: &[EventChange]) -> Event {
        assert_eq!(changes.len(), 1, "{changes:?}");
        match &changes[0] {
            EventChange::Closed(e) => e.clone(),
            other => panic!("expected closed, got {other:?}"),
        }
    }

    /// Opens with `bad`, stays silent with `bad` again and closes with `good`.
    fn opens_once_and_closes(
        bad: impl Fn(&mut Snapshot),
        good: impl Fn(&mut Snapshot),
        kind: EventKind,
    ) -> Event {
        let mut open = Vec::new();
        let mut s = empty(T0);
        bad(&mut s);
        let event = opened(&step(&mut open, &s));
        assert_eq!(event.kind, kind);
        assert_eq!(event.started_at, T0);
        assert_eq!(event.ended_at, None);

        let mut s = empty(T0 + STEP);
        bad(&mut s);
        assert!(step(&mut open, &s).is_empty());

        let mut s = empty(T0 + 2 * STEP);
        good(&mut s);
        let end = closed(&step(&mut open, &s));
        assert_eq!(end.started_at, T0);
        assert_eq!(end.ended_at, Some(T0 + 2 * STEP));
        assert_eq!(end.text, event.text);
        event
    }

    #[test]
    fn alert_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.alerts = vec![alert("Lüfter laut")],
            |_| {},
            EventKind::Alert,
        );
        assert_eq!(e.subject.as_deref(), Some("Lüfter laut"));
        assert_eq!(e.level, Health::Warning);
    }

    #[test]
    fn two_alerts_are_two_events() {
        let mut open = Vec::new();
        let mut s = empty(T0);
        s.alerts = vec![alert("A"), alert("B"), alert("A")];
        let changes = step(&mut open, &s);
        assert_eq!(changes.len(), 2);
        s.observed_at += STEP;
        s.alerts = vec![alert("B")];
        let end = closed(&step(&mut open, &s));
        assert_eq!(end.subject.as_deref(), Some("A"));
    }

    #[test]
    fn disk_temperature_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.disks = vec![disk(Some(52.0), None)],
            |s| s.disks = vec![disk(Some(40.0), None)],
            EventKind::DiskTemperature,
        );
        assert_eq!(e.subject.as_deref(), Some("disk-3"));
        assert_eq!(e.text, "Platte 3 bei 52 °C");
    }

    #[test]
    fn disk_health_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.disks = vec![disk(None, Some(Health::Critical))],
            |s| s.disks = vec![disk(None, Some(Health::Ok))],
            EventKind::DiskHealth,
        );
        assert_eq!(e.level, Health::Critical);
    }

    #[test]
    fn volume_health_opens_once_and_closes() {
        opens_once_and_closes(
            |s| s.volumes = vec![volume(Some(100), Some(Health::Warning))],
            |s| s.volumes = vec![volume(Some(100), Some(Health::Ok))],
            EventKind::VolumeHealth,
        );
    }

    #[test]
    fn volume_fill_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.volumes = vec![volume(Some(850), Some(Health::Ok))],
            |s| s.volumes = vec![volume(Some(500), Some(Health::Ok))],
            EventKind::VolumeFill,
        );
        assert_eq!(e.text, "DataVol1 bei 85 %");
    }

    #[test]
    fn fan_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.fans = vec![fan(Some(Health::Warning))],
            |s| s.fans = vec![fan(Some(Health::Ok))],
            EventKind::FanHealth,
        );
        assert_eq!(e.subject.as_deref(), Some("Lüfter 1"));
    }

    #[test]
    fn ups_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.ups = Some(ups(Some(UpsState::OnBattery))),
            |s| s.ups = Some(ups(Some(UpsState::Online))),
            EventKind::UpsOnBattery,
        );
        assert_eq!(e.level, Health::Critical);
        assert_eq!(e.subject, None);
    }

    #[test]
    fn failed_backup_opens_once_and_closes() {
        opens_once_and_closes(
            |s| s.backups = vec![backup(Some(T0), Some(BackupResult::Error))],
            |s| s.backups = vec![backup(Some(T0 + STEP), Some(BackupResult::Ok))],
            EventKind::BackupFailed,
        );
    }

    #[test]
    fn running_backup_keeps_its_event_open() {
        let running = |s: &mut Snapshot| {
            s.backups = vec![backup(Some(T0 + STEP), Some(BackupResult::Running))]
        };
        let bads: [(EventKind, Change); 2] = [
            (EventKind::BackupFailed, |s| {
                s.backups = vec![backup(Some(T0), Some(BackupResult::Error))]
            }),
            (EventKind::BackupOverdue, |s| {
                s.backups = vec![backup(Some(T0 - 3 * 86_400), Some(BackupResult::Ok))]
            }),
        ];
        for (kind, bad) in bads {
            let mut open = Vec::new();
            let mut s = empty(T0);
            bad(&mut s);
            assert_eq!(opened(&step(&mut open, &s)).kind, kind);

            // A new run starts: still open.
            let mut s = empty(T0 + STEP);
            running(&mut s);
            assert!(
                step(&mut open, &s).is_empty(),
                "{kind:?}: running stays open"
            );

            // It fails again: the same event, no second one.
            let mut s = empty(T0 + 2 * STEP);
            s.backups = vec![backup(Some(T0 + STEP), Some(BackupResult::Error))];
            let changes = step(&mut open, &s);
            if kind == EventKind::BackupFailed {
                assert!(changes.is_empty(), "{changes:?}");
            } else {
                // Overdue gives way to failed: the overdue event closes.
                assert_eq!(changes.len(), 2, "{changes:?}");
            }

            // It succeeds: closed.
            let mut s = empty(T0 + 3 * STEP);
            s.backups = vec![backup(Some(T0 + 3 * STEP), Some(BackupResult::Ok))];
            let changes = step(&mut open, &s);
            assert!(
                changes.iter().all(|c| matches!(c, EventChange::Closed(_))),
                "{changes:?}"
            );
            assert!(open.iter().all(|e| !e.is_open()));
        }
    }

    #[test]
    fn overdue_backup_opens_once_and_closes() {
        let e = opens_once_and_closes(
            |s| s.backups = vec![backup(Some(T0 - 3 * 86_400), Some(BackupResult::Ok))],
            |s| s.backups = vec![backup(Some(T0), Some(BackupResult::Ok))],
            EventKind::BackupOverdue,
        );
        assert_eq!(e.subject.as_deref(), Some("b1"));
    }

    #[test]
    fn disk_temperature_hysteresis() {
        let mut open = Vec::new();
        let mut s = empty(T0);
        s.disks = vec![disk(Some(51.0), None)];
        opened(&step(&mut open, &s));

        for (i, temp) in [49.9, 48.0, 47.0, 50.5].into_iter().enumerate() {
            let mut s = empty(T0 + (i as i64 + 1) * STEP);
            s.disks = vec![disk(Some(temp), None)];
            assert!(step(&mut open, &s).is_empty(), "{temp} °C");
        }

        let mut s = empty(T0 + 10 * STEP);
        s.disks = vec![disk(Some(46.0), None)];
        closed(&step(&mut open, &s));

        // A new rise is a new event.
        let mut s = empty(T0 + 11 * STEP);
        s.disks = vec![disk(Some(50.0), None)];
        assert_eq!(opened(&step(&mut open, &s)).started_at, T0 + 11 * STEP);
    }

    #[test]
    fn fill_hysteresis() {
        let mut open = Vec::new();
        let mut s = empty(T0);
        s.volumes = vec![volume(Some(810), None)];
        opened(&step(&mut open, &s));

        let mut s = empty(T0 + STEP);
        s.volumes = vec![volume(Some(795), None)];
        assert!(step(&mut open, &s).is_empty(), "79.5 % stays open");

        let mut s = empty(T0 + 2 * STEP);
        s.volumes = vec![volume(Some(789), None)];
        closed(&step(&mut open, &s));
    }

    #[test]
    fn fill_raises_without_a_second_event() {
        let mut open = Vec::new();
        let mut s = empty(T0);
        s.volumes = vec![volume(Some(850), None)];
        let first = opened(&step(&mut open, &s));
        assert_eq!(first.level, Health::Warning);

        let mut s = empty(T0 + STEP);
        s.volumes = vec![volume(Some(920), None)];
        let changes = step(&mut open, &s);
        assert_eq!(changes.len(), 1);
        let EventChange::Raised(raised) = &changes[0] else {
            panic!("expected raised, got {changes:?}");
        };
        assert_eq!(raised.level, Health::Critical);
        assert_eq!(raised.text, "DataVol1 bei 92 %");
        assert_eq!(raised.started_at, T0);
        assert_eq!(raised.ended_at, None);
        assert_eq!(open.len(), 1);

        // Back to 85 %: the event stays critical and nothing changes.
        let mut s = empty(T0 + 2 * STEP);
        s.volumes = vec![volume(Some(850), None)];
        assert!(step(&mut open, &s).is_empty());
        assert_eq!(open[0].level, Health::Critical);
        assert_eq!(open[0].text, "DataVol1 bei 92 %");
    }

    #[test]
    fn unknown_value_keeps_open_and_missing_part_closes() {
        let cases: [(EventKind, Change, Change); 8] = [
            (
                EventKind::DiskTemperature,
                |s| s.disks = vec![disk(Some(55.0), None)],
                |s| s.disks = vec![disk(None, None)],
            ),
            (
                EventKind::DiskHealth,
                |s| s.disks = vec![disk(None, Some(Health::Warning))],
                |s| s.disks = vec![disk(None, None)],
            ),
            (
                EventKind::VolumeFill,
                |s| s.volumes = vec![volume(Some(900), None)],
                |s| s.volumes = vec![volume(None, None)],
            ),
            (
                EventKind::VolumeHealth,
                |s| s.volumes = vec![volume(None, Some(Health::Critical))],
                |s| s.volumes = vec![volume(None, None)],
            ),
            (
                EventKind::FanHealth,
                |s| s.fans = vec![fan(Some(Health::Warning))],
                |s| s.fans = vec![fan(None)],
            ),
            (
                EventKind::UpsOnBattery,
                |s| s.ups = Some(ups(Some(UpsState::OnBattery))),
                |s| s.ups = Some(ups(None)),
            ),
            (
                EventKind::BackupFailed,
                |s| s.backups = vec![backup(Some(T0), Some(BackupResult::Error))],
                |s| s.backups = vec![backup(Some(T0), None)],
            ),
            (
                EventKind::BackupOverdue,
                |s| s.backups = vec![backup(Some(T0 - 5 * 86_400), Some(BackupResult::Ok))],
                |s| s.backups = vec![backup(None, Some(BackupResult::Ok))],
            ),
        ];
        for (kind, bad, unknown) in cases {
            let mut open = Vec::new();
            let mut s = empty(T0);
            bad(&mut s);
            assert_eq!(opened(&step(&mut open, &s)).kind, kind);

            let mut s = empty(T0 + STEP);
            unknown(&mut s);
            assert!(
                step(&mut open, &s).is_empty(),
                "{kind:?}: unknown stays open"
            );

            // The part is gone.
            let s = empty(T0 + 2 * STEP);
            assert_eq!(closed(&step(&mut open, &s)).kind, kind);
        }
    }

    #[test]
    fn other_nas_and_closed_events_are_ignored() {
        let mut s = empty(T0);
        s.disks = vec![disk(Some(52.0), None)];
        let mine = opened(&detect_events(&[], &s, &Thresholds::default()));

        let other = Event {
            nas_id: "nas-2".into(),
            ..mine.clone()
        };
        let done = Event {
            ended_at: Some(T0 - STEP),
            ..mine.clone()
        };
        // Neither counts as open for nas-1: the disk opens a new event.
        let changes = detect_events(&[other.clone(), done.clone()], &s, &Thresholds::default());
        assert_eq!(opened(&changes), mine);

        // And neither is closed by a snapshot without findings.
        let changes = detect_events(&[other, done], &empty(T0), &Thresholds::default());
        assert!(changes.is_empty());
    }

    #[test]
    fn first_run_opens_every_finding_in_order() {
        let mut s = empty(T0);
        s.disks = vec![disk(Some(52.0), None)];
        s.volumes = vec![volume(Some(950), None)];
        s.ups = Some(ups(Some(UpsState::OnBattery)));
        let changes = detect_events(&[], &s, &Thresholds::default());
        let kinds: Vec<_> = changes
            .iter()
            .map(|c| match c {
                EventChange::Opened(e) => e.kind,
                other => panic!("expected opened, got {other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                EventKind::VolumeFill,
                EventKind::UpsOnBattery,
                EventKind::DiskTemperature
            ]
        );
        assert!(detect_events(&[], &empty(T0), &Thresholds::default()).is_empty());
    }

    #[test]
    fn opened_and_raised_before_closed() {
        let mut open = Vec::new();
        let mut s = empty(T0);
        s.ups = Some(ups(Some(UpsState::OnBattery)));
        s.volumes = vec![volume(Some(850), None)];
        step(&mut open, &s);

        let mut s = empty(T0 + STEP);
        s.volumes = vec![volume(Some(950), None)];
        s.disks = vec![disk(Some(52.0), None)];
        let changes = step(&mut open, &s);
        let kinds: Vec<_> = changes
            .iter()
            .map(|c| match c {
                EventChange::Opened(e) => ("opened", e.kind),
                EventChange::Raised(e) => ("raised", e.kind),
                EventChange::Closed(e) => ("closed", e.kind),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("raised", EventKind::VolumeFill),
                ("opened", EventKind::DiskTemperature),
                ("closed", EventKind::UpsOnBattery),
            ]
        );
    }

    #[test]
    fn unreachable_after_15_minutes() {
        let nas = empty(T0).nas;
        assert_eq!(detect_unreachable(&[], &nas, Some(T0), T0 + 899), None);
        assert_eq!(detect_unreachable(&[], &nas, None, T0 + 10_000), None);

        let change = detect_unreachable(&[], &nas, Some(T0), T0 + 900).expect("unreachable");
        let EventChange::Opened(event) = &change else {
            panic!("expected opened, got {change:?}");
        };
        assert_eq!(event.kind, EventKind::Unreachable);
        assert_eq!(event.level, Health::Critical);
        assert_eq!(event.subject, None);
        assert_eq!(event.text, "Keller nicht erreichbar");
        assert_eq!(event.started_at, T0);

        // Not a second time while it is open.
        let mut open = vec![event.clone()];
        assert_eq!(detect_unreachable(&open, &nas, Some(T0), T0 + 5000), None);

        // The next snapshot closes it.
        let end = closed(&step(&mut open, &empty(T0 + 6000)));
        assert_eq!(end.kind, EventKind::Unreachable);
        assert_eq!(end.ended_at, Some(T0 + 6000));
    }

    #[test]
    fn unreachable_name_falls_back() {
        let mut nas = empty(T0).nas;
        nas.name = None;
        let text = |nas: &Nas| {
            detect_unreachable(&[], nas, Some(T0), T0 + 900)
                .expect("unreachable")
                .event()
                .text
                .clone()
        };
        assert_eq!(text(&nas), "keller-nas nicht erreichbar");
        nas.hostname = None;
        assert_eq!(text(&nas), "https://nas.example:8080 nicht erreichbar");
    }

    #[test]
    fn unreachable_of_another_nas_does_not_count() {
        let nas = empty(T0).nas;
        let mut other = nas.clone();
        other.id = "nas-2".into();
        let open = vec![
            detect_unreachable(&[], &other, Some(T0), T0 + 900)
                .expect("unreachable")
                .event()
                .clone(),
        ];
        assert!(detect_unreachable(&open, &nas, Some(T0), T0 + 900).is_some());
    }

    #[test]
    fn event_kind_from_every_finding_kind() {
        assert_eq!(EventKind::from(FindingKind::Alert), EventKind::Alert);
        assert_eq!(
            EventKind::from(FindingKind::BackupOverdue),
            EventKind::BackupOverdue
        );
        assert_eq!(
            EventKind::from(FindingKind::VolumeFill),
            EventKind::VolumeFill
        );
    }

    #[test]
    fn change_survives_json() {
        let change = EventChange::Closed(Event {
            nas_id: "nas-1".into(),
            kind: EventKind::Unreachable,
            subject: None,
            level: Health::Critical,
            text: "Keller nicht erreichbar".into(),
            started_at: T0,
            ended_at: Some(T0 + 1080),
        });
        let json = serde_json::to_string(&change).expect("change serializes");
        assert!(json.contains("\"closed\""), "{json}");
        assert!(json.contains("\"unreachable\""), "{json}");
        let back: EventChange = serde_json::from_str(&json).expect("change deserializes");
        assert_eq!(back, change);
    }
}
