//! Polling rules without a runtime: who is due, when, and what a result does.

use std::collections::BTreeMap;

use nas_model::{ProviderError, Snapshot};

/// Shortest interval between two automatic queries of one NAS, seconds.
pub const MIN_INTERVAL_SECS: i64 = 300;

/// Shortest pause between the last query and a manual refresh, seconds.
pub const MANUAL_PAUSE_SECS: i64 = 30;

/// What the planner knows about one NAS.
#[derive(Debug, Clone, PartialEq)]
pub struct PollState {
    pub nas_id: String,
    /// Last successful snapshot; stays when a later query fails.
    pub snapshot: Option<Snapshot>,
    /// Error of the last query, `None` after a success.
    pub error: Option<ProviderError>,
    /// When the last query failed, Unix seconds UTC.
    pub error_at: Option<i64>,
    /// When the last query started, Unix seconds UTC.
    pub last_attempt_at: Option<i64>,
    /// No more queries until [`Schedule::unblock`].
    pub blocked: bool,
    /// A query is in flight.
    pub running: bool,
}

/// Answer to a manual refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manual {
    /// Query now.
    Go,
    /// The last query is too recent; not before `retry_at` (Unix seconds UTC).
    TooSoon { retry_at: i64 },
    /// Stopped after a rejected login, second factor or changed certificate.
    Blocked,
    /// A query is in flight already.
    Running,
    /// No NAS with this id.
    Unknown,
}

#[derive(Debug)]
struct Entry {
    state: PollState,
    generation: u64,
    /// Generation of the query in flight; may be an older one after the
    /// NAS was replaced, so that no second query starts beside it.
    in_flight: Option<u64>,
}

/// Polling state of all NAS, with the rules of the planner.
#[derive(Debug)]
pub struct Schedule {
    interval: i64,
    entries: BTreeMap<String, Entry>,
    next_generation: u64,
}

impl Schedule {
    /// Schedule with `interval_secs` between automatic queries; below
    /// [`MIN_INTERVAL_SECS`] counts as the minimum.
    pub fn new(interval_secs: i64) -> Self {
        Self {
            interval: interval_secs.max(MIN_INTERVAL_SECS),
            entries: BTreeMap::new(),
            next_generation: 0,
        }
    }

    /// Interval between automatic queries, seconds.
    pub fn interval(&self) -> i64 {
        self.interval
    }

    /// New interval; below [`MIN_INTERVAL_SECS`] counts as the minimum.
    pub fn set_interval(&mut self, interval_secs: i64) {
        self.interval = interval_secs.max(MIN_INTERVAL_SECS);
    }

    /// Registers a NAS with what the app remembers across launches: the
    /// start of the last query and whether it is blocked.
    ///
    /// Replacing an existing NAS keeps only its snapshot; a query still in
    /// flight for the old entry is discarded when it finishes, and until
    /// then the new entry counts as running.
    pub fn add(&mut self, nas_id: &str, last_attempt_at: Option<i64>, blocked: bool) {
        let (snapshot, in_flight) = self.entries.remove(nas_id).map_or((None, None), |entry| {
            (entry.state.snapshot, entry.in_flight)
        });
        self.next_generation += 1;
        self.entries.insert(
            nas_id.to_owned(),
            Entry {
                state: PollState {
                    nas_id: nas_id.to_owned(),
                    snapshot,
                    error: None,
                    error_at: None,
                    last_attempt_at,
                    blocked,
                    running: in_flight.is_some(),
                },
                generation: self.next_generation,
                in_flight,
            },
        );
    }

    /// Removes a NAS; `false` when it was not registered.
    pub fn remove(&mut self, nas_id: &str) -> bool {
        self.entries.remove(nas_id).is_some()
    }

    fn is_due(&self, state: &PollState, now: i64) -> bool {
        if state.blocked || state.running {
            return false;
        }
        match state.last_attempt_at {
            None => true,
            // A last query in the future means the clock went back.
            Some(last) => last > now || now.saturating_sub(last) >= self.interval,
        }
    }

    /// Ids of the NAS due for an automatic query, sorted.
    pub fn due(&self, now: i64) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| self.is_due(&entry.state, now))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// When the next automatic query is due: `now` when one is due already,
    /// `None` when no NAS can be queried (none registered, all blocked or
    /// running).
    pub fn next_due_at(&self, now: i64) -> Option<i64> {
        self.entries
            .values()
            .map(|entry| &entry.state)
            .filter(|state| !state.blocked && !state.running)
            .map(|state| {
                if self.is_due(state, now) {
                    now
                } else {
                    state
                        .last_attempt_at
                        .map_or(now, |last| last.saturating_add(self.interval))
                }
            })
            .min()
    }

    /// Whether a manual refresh may query now.
    pub fn manual(&self, nas_id: &str, now: i64) -> Manual {
        let Some(entry) = self.entries.get(nas_id) else {
            return Manual::Unknown;
        };
        let state = &entry.state;
        if state.blocked {
            return Manual::Blocked;
        }
        if state.running {
            return Manual::Running;
        }
        if let Some(last) = state.last_attempt_at {
            let since = now.saturating_sub(last);
            if (0..MANUAL_PAUSE_SECS).contains(&since) {
                return Manual::TooSoon {
                    retry_at: last.saturating_add(MANUAL_PAUSE_SECS),
                };
            }
        }
        Manual::Go
    }

    /// Starts a query: marks it running and returns the generation to pass
    /// to [`Schedule::finish`]. `None` for an unknown, blocked or running
    /// NAS. The next automatic query follows one interval after `now`, also
    /// for a manual one.
    pub fn begin(&mut self, nas_id: &str, now: i64) -> Option<u64> {
        let entry = self.entries.get_mut(nas_id)?;
        if entry.state.blocked || entry.state.running {
            return None;
        }
        entry.state.last_attempt_at = Some(now);
        entry.state.running = true;
        entry.in_flight = Some(entry.generation);
        Some(entry.generation)
    }

    /// Ends a query started with [`Schedule::begin`] that gave no result
    /// (cancelled or panicked): only the running mark goes, the start time
    /// stays. `false` when that query is not in flight.
    pub fn abandon(&mut self, nas_id: &str, generation: u64) -> bool {
        let Some(entry) = self.entries.get_mut(nas_id) else {
            return false;
        };
        if entry.in_flight != Some(generation) {
            return false;
        }
        entry.in_flight = None;
        entry.state.running = false;
        true
    }

    /// Records the result of a query started with [`Schedule::begin`].
    ///
    /// `None`, and nothing changes, when the NAS was removed or replaced
    /// meanwhile. A success replaces the snapshot and clears the error; an
    /// error keeps the snapshot, records time and error and blocks the NAS
    /// when the error stops polling.
    pub fn finish(
        &mut self,
        nas_id: &str,
        generation: u64,
        now: i64,
        result: Result<Snapshot, ProviderError>,
    ) -> Option<PollState> {
        let entry = self.entries.get_mut(nas_id)?;
        if entry.in_flight != Some(generation) {
            return None;
        }
        entry.in_flight = None;
        entry.state.running = false;
        if entry.generation != generation {
            return None;
        }
        let state = &mut entry.state;
        match result {
            Ok(snapshot) => {
                state.snapshot = Some(snapshot);
                state.error = None;
                state.error_at = None;
            }
            Err(error) => {
                state.blocked = error.stops_polling();
                state.error = Some(error);
                state.error_at = Some(now);
            }
        }
        Some(state.clone())
    }

    /// Lifts a block after the user acted; the NAS is due at once. `false`
    /// when the NAS is unknown or not blocked.
    pub fn unblock(&mut self, nas_id: &str) -> bool {
        let Some(entry) = self.entries.get_mut(nas_id) else {
            return false;
        };
        let state = &mut entry.state;
        if !state.blocked {
            return false;
        }
        state.blocked = false;
        state.error = None;
        state.error_at = None;
        state.last_attempt_at = None;
        true
    }

    /// State of one NAS.
    pub fn state(&self, nas_id: &str) -> Option<PollState> {
        self.entries.get(nas_id).map(|entry| entry.state.clone())
    }

    /// States of all NAS, sorted by id.
    pub fn states(&self) -> Vec<PollState> {
        self.entries
            .values()
            .map(|entry| entry.state.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use nas_model::{Nas, ProviderKind, SystemStatus};

    use super::*;

    const T0: i64 = 1_800_000_000;

    fn snapshot(observed_at: i64) -> Snapshot {
        Snapshot {
            nas: Nas {
                id: "nas-1".into(),
                provider: ProviderKind::Qnap,
                name: None,
                base_url: "https://nas.example".into(),
                model: None,
                firmware: None,
                hostname: None,
            },
            observed_at,
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

    fn poll(schedule: &mut Schedule, now: i64, result: Result<Snapshot, ProviderError>) {
        let generation = schedule.begin("nas-1", now).expect("query starts");
        schedule
            .finish("nas-1", generation, now, result)
            .expect("result recorded");
    }

    #[test]
    fn interval_below_minimum_counts_as_minimum() {
        let mut schedule = Schedule::new(60);
        assert_eq!(schedule.interval(), MIN_INTERVAL_SECS);
        schedule.set_interval(600);
        assert_eq!(schedule.interval(), 600);
        schedule.set_interval(0);
        assert_eq!(schedule.interval(), MIN_INTERVAL_SECS);
    }

    #[test]
    fn new_nas_is_due_and_then_waits_one_interval() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        assert_eq!(schedule.due(T0), vec!["nas-1".to_owned()]);
        assert_eq!(schedule.next_due_at(T0), Some(T0));
        poll(&mut schedule, T0, Ok(snapshot(T0)));
        assert!(schedule.due(T0 + 299).is_empty());
        assert_eq!(schedule.next_due_at(T0 + 10), Some(T0 + 300));
        assert_eq!(schedule.due(T0 + 300), vec!["nas-1".to_owned()]);
    }

    #[test]
    fn running_query_is_not_due_and_not_restarted() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        let generation = schedule.begin("nas-1", T0);
        assert!(generation.is_some());
        assert!(schedule.due(T0 + 600).is_empty());
        assert_eq!(schedule.next_due_at(T0 + 600), None);
        assert_eq!(schedule.begin("nas-1", T0 + 600), None);
        assert_eq!(schedule.manual("nas-1", T0 + 600), Manual::Running);
    }

    #[test]
    fn timeout_keeps_snapshot_with_time_and_text() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        poll(&mut schedule, T0, Ok(snapshot(T0)));
        poll(&mut schedule, T0 + 300, Err(ProviderError::Timeout));

        let state = schedule.state("nas-1").expect("registered");
        assert_eq!(state.snapshot, Some(snapshot(T0)));
        assert_eq!(state.error_at, Some(T0 + 300));
        assert_eq!(
            state.error.map(|e| e.to_string()).as_deref(),
            Some("Zeitüberschreitung bei der Abfrage")
        );
        assert!(!state.blocked);
        // Next attempt at the regular time, not earlier.
        assert!(schedule.due(T0 + 599).is_empty());
        assert_eq!(schedule.next_due_at(T0 + 301), Some(T0 + 600));

        poll(&mut schedule, T0 + 600, Ok(snapshot(T0 + 600)));
        let state = schedule.state("nas-1").expect("registered");
        assert_eq!(state.error, None);
        assert_eq!(state.error_at, None);
        assert_eq!(state.snapshot, Some(snapshot(T0 + 600)));
    }

    #[test]
    fn login_second_factor_and_certificate_block_until_unblocked() {
        let errors = [
            ProviderError::LoginFailed,
            ProviderError::SecondFactorRequired,
            ProviderError::CertificateChanged {
                old: "aa".into(),
                new: "bb".into(),
            },
        ];
        for error in errors {
            let mut schedule = Schedule::new(300);
            schedule.add("nas-1", None, false);
            poll(&mut schedule, T0, Err(error.clone()));

            let state = schedule.state("nas-1").expect("registered");
            assert!(state.blocked, "{error}");
            assert_eq!(state.error, Some(error.clone()));
            assert!(schedule.due(T0 + 86_400).is_empty(), "{error}");
            assert_eq!(schedule.next_due_at(T0 + 86_400), None, "{error}");
            assert_eq!(schedule.manual("nas-1", T0 + 86_400), Manual::Blocked);
            assert_eq!(schedule.begin("nas-1", T0 + 86_400), None);

            assert!(schedule.unblock("nas-1"));
            let state = schedule.state("nas-1").expect("registered");
            assert!(!state.blocked);
            assert_eq!(state.error, None);
            assert_eq!(state.last_attempt_at, None);
            assert_eq!(schedule.due(T0 + 1), vec!["nas-1".to_owned()]);
        }
    }

    #[test]
    fn unblock_leaves_a_nas_that_is_not_blocked_alone() {
        let mut schedule = Schedule::new(300);
        assert!(!schedule.unblock("nas-1"));
        schedule.add("nas-1", Some(T0), false);
        assert!(!schedule.unblock("nas-1"));
        assert!(schedule.due(T0 + 1).is_empty());
    }

    #[test]
    fn manual_refresh_waits_30_seconds_and_moves_the_next_automatic_query() {
        let mut schedule = Schedule::new(300);
        assert_eq!(schedule.manual("nas-1", T0), Manual::Unknown);
        schedule.add("nas-1", None, false);
        assert_eq!(schedule.manual("nas-1", T0), Manual::Go);
        poll(&mut schedule, T0, Ok(snapshot(T0)));

        assert_eq!(
            schedule.manual("nas-1", T0 + 29),
            Manual::TooSoon { retry_at: T0 + 30 }
        );
        assert_eq!(schedule.manual("nas-1", T0 + 30), Manual::Go);
        poll(&mut schedule, T0 + 30, Ok(snapshot(T0 + 30)));
        assert!(schedule.due(T0 + 300).is_empty());
        assert_eq!(schedule.next_due_at(T0 + 31), Some(T0 + 330));
        assert_eq!(schedule.due(T0 + 330), vec!["nas-1".to_owned()]);
    }

    #[test]
    fn clock_going_back_makes_the_nas_due() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", Some(T0), false);
        assert_eq!(schedule.due(T0 - 3600), vec!["nas-1".to_owned()]);
        assert_eq!(schedule.next_due_at(T0 - 3600), Some(T0 - 3600));
        assert_eq!(schedule.manual("nas-1", T0 - 3600), Manual::Go);
    }

    #[test]
    fn result_of_an_old_generation_changes_nothing() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        let old = schedule.begin("nas-1", T0).expect("query starts");
        schedule.add("nas-1", Some(T0), false);
        assert_eq!(
            schedule.finish("nas-1", old, T0 + 1, Err(ProviderError::LoginFailed)),
            None
        );
        let state = schedule.state("nas-1").expect("registered");
        assert!(!state.blocked);
        assert_eq!(state.error, None);

        assert!(schedule.remove("nas-1"));
        assert!(!schedule.remove("nas-1"));
        assert_eq!(
            schedule.finish("nas-1", old, T0 + 1, Ok(snapshot(T0))),
            None
        );
    }

    #[test]
    fn replacing_during_a_query_starts_no_second_one() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        let old = schedule.begin("nas-1", T0).expect("query starts");
        schedule.add("nas-1", None, false);
        assert!(schedule.due(T0).is_empty());
        assert_eq!(schedule.manual("nas-1", T0), Manual::Running);
        assert_eq!(schedule.begin("nas-1", T0), None);

        assert_eq!(
            schedule.finish("nas-1", old, T0 + 5, Ok(snapshot(T0))),
            None
        );
        let state = schedule.state("nas-1").expect("registered");
        assert!(!state.running);
        assert_eq!(state.snapshot, None);
        assert_eq!(schedule.due(T0 + 5), vec!["nas-1".to_owned()]);
    }

    #[test]
    fn abandoned_query_frees_the_nas() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        let generation = schedule.begin("nas-1", T0).expect("query starts");
        assert!(schedule.abandon("nas-1", generation));
        assert!(!schedule.abandon("nas-1", generation));
        let state = schedule.state("nas-1").expect("registered");
        assert!(!state.running);
        assert_eq!(state.last_attempt_at, Some(T0));
        assert_eq!(schedule.due(T0 + 300), vec!["nas-1".to_owned()]);
        assert_eq!(
            schedule.finish("nas-1", generation, T0 + 1, Ok(snapshot(T0))),
            None
        );
    }

    #[test]
    fn absurd_remembered_time_does_not_overflow() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", Some(i64::MIN), false);
        assert_eq!(schedule.due(T0), vec!["nas-1".to_owned()]);
        assert_eq!(schedule.manual("nas-1", T0), Manual::Go);
        schedule.add("nas-2", Some(i64::MAX), false);
        assert_eq!(schedule.manual("nas-2", T0), Manual::Go);
    }

    #[test]
    fn replacing_a_nas_keeps_only_the_snapshot() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", None, false);
        poll(&mut schedule, T0, Ok(snapshot(T0)));
        poll(&mut schedule, T0 + 300, Err(ProviderError::LoginFailed));
        schedule.add("nas-1", None, false);
        let state = schedule.state("nas-1").expect("registered");
        assert_eq!(state.snapshot, Some(snapshot(T0)));
        assert_eq!(state.error, None);
        assert!(!state.blocked);
    }

    #[test]
    fn remembered_last_query_and_block_are_respected() {
        let mut schedule = Schedule::new(300);
        schedule.add("nas-1", Some(T0 - 100), false);
        assert!(schedule.due(T0 + 199).is_empty());
        assert_eq!(schedule.due(T0 + 200), vec!["nas-1".to_owned()]);

        schedule.add("nas-2", None, true);
        assert!(!schedule.due(T0 + 86_400).contains(&"nas-2".to_owned()));
        assert_eq!(schedule.manual("nas-2", T0), Manual::Blocked);
        let ids: Vec<_> = schedule.states().into_iter().map(|s| s.nas_id).collect();
        assert_eq!(ids, ["nas-1", "nas-2"]);
    }
}
