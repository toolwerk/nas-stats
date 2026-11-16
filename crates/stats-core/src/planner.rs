//! The planner: runs the [`Schedule`] on tokio and queries the providers.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nas_model::Provider;
use tokio::sync::Notify;

use crate::schedule::{Manual, PollState, Schedule};

/// Receives every finished query, never a skipped or discarded one.
///
/// History, events and the callback to the apps attach here. Called from
/// the task that ran the query, one call at a time and in the order the
/// queries finished. A NAS removed or replaced during its query is not
/// reported. The sink may read the planner, but must not call
/// [`Planner::add`] or [`Planner::remove`], and must not wait for a thread
/// that may call them (e.g. a synchronous hand-over to the main thread);
/// both would deadlock.
pub trait PollSink: Send + Sync {
    fn polled(&self, state: &PollState);
}

/// Current time, Unix seconds UTC. Tests pass a simulated one.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    })
}

/// Longest sleep of [`Planner::run`], seconds. Catches a clock that was
/// set or a machine that slept.
pub const MAX_SLEEP_SECS: u64 = 60;

/// A started query. Created right after [`Schedule::begin`]; when it is
/// dropped without a recorded result (cancelled future, task never polled,
/// panic in provider or clock) it releases the running mark.
struct InFlight {
    planner: Arc<Planner>,
    nas_id: String,
    generation: u64,
    provider: Arc<dyn Provider>,
    done: bool,
}

impl InFlight {
    /// Runs the query and hands the result to the sink.
    async fn run(mut self) -> Option<PollState> {
        let result = self.provider.snapshot().await;
        let planner = Arc::clone(&self.planner);
        let now = planner.now();
        let report = planner.report_lock();
        let state = planner
            .lock()
            .schedule
            .finish(&self.nas_id, self.generation, now, result);
        self.done = true;
        if let Some(state) = &state {
            planner.sink.polled(state);
        }
        drop(report);
        planner.wake.notify_one();
        state
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if !self.done {
            self.planner
                .lock()
                .schedule
                .abandon(&self.nas_id, self.generation);
            self.planner.wake.notify_one();
        }
    }
}

struct Inner {
    schedule: Schedule,
    providers: BTreeMap<String, Arc<dyn Provider>>,
}

/// Queries every registered NAS by the rules of [`Schedule`].
///
/// [`Planner::run`] and [`Planner::refresh_now`] need a tokio runtime with
/// the time driver enabled.
pub struct Planner {
    inner: Mutex<Inner>,
    /// Held from recording a result until the sink has it, and by `add` and
    /// `remove`, so the sink sees results in order and none of a NAS that
    /// was removed or replaced meanwhile. Always taken before `inner`.
    report: Mutex<()>,
    clock: Clock,
    sink: Arc<dyn PollSink>,
    wake: Notify,
    stopped: AtomicBool,
}

impl Planner {
    /// Planner with `interval_secs` between automatic queries (at least
    /// [`crate::MIN_INTERVAL_SECS`]).
    pub fn new(interval_secs: i64, clock: Clock, sink: Arc<dyn PollSink>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                schedule: Schedule::new(interval_secs),
                providers: BTreeMap::new(),
            }),
            report: Mutex::new(()),
            clock,
            sink,
            wake: Notify::new(),
            stopped: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn report_lock(&self) -> MutexGuard<'_, ()> {
        self.report.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Registers or replaces a NAS, with the start of its last query and
    /// its block as the app remembers them across launches.
    pub fn add(
        &self,
        nas_id: &str,
        provider: Arc<dyn Provider>,
        last_attempt_at: Option<i64>,
        blocked: bool,
    ) {
        {
            let _report = self.report_lock();
            let mut inner = self.lock();
            inner.schedule.add(nas_id, last_attempt_at, blocked);
            inner.providers.insert(nas_id.to_owned(), provider);
        }
        self.wake.notify_one();
    }

    /// Removes a NAS; a query in flight is discarded. `false` when unknown.
    pub fn remove(&self, nas_id: &str) -> bool {
        let _report = self.report_lock();
        let mut inner = self.lock();
        inner.providers.remove(nas_id);
        inner.schedule.remove(nas_id)
    }

    /// Interval between automatic queries, seconds.
    pub fn interval(&self) -> i64 {
        self.lock().schedule.interval()
    }

    /// New interval (at least [`crate::MIN_INTERVAL_SECS`]).
    pub fn set_interval(&self, interval_secs: i64) {
        self.lock().schedule.set_interval(interval_secs);
        self.wake.notify_one();
    }

    /// Lifts a block after the user acted; the NAS is queried at once.
    /// `false` when it is unknown or not blocked.
    pub fn unblock(&self, nas_id: &str) -> bool {
        let lifted = self.lock().schedule.unblock(nas_id);
        if lifted {
            self.wake.notify_one();
        }
        lifted
    }

    /// State of one NAS.
    pub fn state(&self, nas_id: &str) -> Option<PollState> {
        self.lock().schedule.state(nas_id)
    }

    /// States of all NAS, sorted by id.
    pub fn states(&self) -> Vec<PollState> {
        self.lock().schedule.states()
    }

    /// Starts a query if the schedule allows it.
    fn begin(self: &Arc<Self>, nas_id: &str, now: i64) -> Option<InFlight> {
        let mut inner = self.lock();
        let provider = inner.providers.get(nas_id).cloned()?;
        let generation = inner.schedule.begin(nas_id, now)?;
        Some(InFlight {
            planner: Arc::clone(self),
            nas_id: nas_id.to_owned(),
            generation,
            provider,
            done: false,
        })
    }

    /// Manual refresh: queries now unless the last query is less than
    /// [`crate::MANUAL_PAUSE_SECS`] ago, a query is in flight or the NAS is
    /// blocked or unknown; then the reason comes back, with the earliest
    /// time for [`Manual::TooSoon`]. A NAS removed or replaced during the
    /// query gives [`Manual::Unknown`]. Dropping the future before it ends
    /// frees the NAS again; the start counts as the last query.
    ///
    /// Works also after [`Planner::stop`]: a manual refresh is the user's
    /// own action.
    pub async fn refresh_now(self: &Arc<Self>, nas_id: &str) -> Result<PollState, Manual> {
        let now = self.now();
        match self.lock().schedule.manual(nas_id, now) {
            Manual::Go => {}
            other => return Err(other),
        }
        let query = self.begin(nas_id, now).ok_or(Manual::Unknown)?;
        query.run().await.ok_or(Manual::Unknown)
    }

    /// Queries every due NAS, each in its own task, until [`Planner::stop`].
    pub async fn run(self: Arc<Self>) {
        while !self.stopped.load(Ordering::SeqCst) {
            let now = self.now();
            let due = self.lock().schedule.due(now);
            for nas_id in due {
                if let Some(query) = self.begin(&nas_id, now) {
                    tokio::spawn(query.run());
                }
            }

            let now = self.now();
            let sleep_secs = match self.lock().schedule.next_due_at(now) {
                Some(at) => u64::try_from(at.saturating_sub(now)).unwrap_or(0),
                None => MAX_SLEEP_SECS,
            }
            .min(MAX_SLEEP_SECS);
            if self.stopped.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {}
                () = self.wake.notified() => {}
            }
        }
    }

    /// Ends [`Planner::run`]. Queries in flight still finish and report.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }
}
