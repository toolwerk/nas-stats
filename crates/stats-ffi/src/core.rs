//! [`NasCore`]: the NAS the app registered, their last state, the planner
//! that queries them and the history archive.
//!
//! The app keeps the list of NAS and the credentials (keychain) and hands
//! every NAS to the core at each start with [`NasCore::add_nas`].
//!
//! Every query runs through the planner of `stats-core`, also a
//! [`NasCore::refresh`] of the app, so two timers never query the same NAS
//! twice. Rules:
//!
//! - A NAS is queried at most once per [`MIN_INTERVAL_SECS`]; in between
//!   [`NasCore::refresh`] returns the last state without a request. A clock
//!   set back counts as the interval passed. [`NasCore::refresh_now`] is the
//!   manual refresh, at the earliest 30 seconds after the last query.
//! - After a rejected login, a required second factor or a changed
//!   certificate the core no longer queries the NAS until the app changes
//!   the credentials or adds the NAS again.
//! - The last snapshot stays after every error.
//! - With [`NasCore::start_planner`] the core queries on its own and calls
//!   the [`StateListener`] after every finished query, also a failed one.
//!
//! Every finished query goes into the history archive: the samples of a
//! snapshot, and the events found with the standard thresholds (disk 50 °C,
//! volume 80 and 90 %), "unreachable" 15 minutes after the last snapshot.
//! An error of the archive never stops a query or the callback. The archive
//! lives in a file ([`NasCore::with_history`]) or in memory.
//!
//! So the app can show the last state after a restart, it saves the
//! snapshot of every NAS with [`NasCore::saved_state`] and hands it back
//! with [`NasCore::restore_state`]; the time of the last query and a block
//! go back with [`NasCore::restore_poll`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nas_http::{Fingerprint, NasClient};
use nas_model::{self as model, BoxFuture, Provider, ProviderError};
use provider_qnap::{QnapClient, QnapProvider};
use stats_core::{
    Clock, EventRecord, HistoryError, HistoryStore, Planner, PollSink, PollState, system_clock,
};

use crate::error::FfiError;
use crate::records::{
    BackupRun, Forecast, HistoryRange, HistorySeries, Metric, NasConfig, NasEvent, NasProblem,
    NasState, ProviderKind, Snapshot, StateListener,
};
use crate::{on_core, runtime};

/// Shortest time between two automatic queries of the same NAS, in seconds.
pub const MIN_INTERVAL_SECS: i64 = stats_core::MIN_INTERVAL_SECS;

/// What the core needs from a connected NAS. The only place that knows the
/// vendors is [`connect`].
pub(crate) trait Backend: Send + Sync {
    fn snapshot(&self) -> BoxFuture<'_, Result<model::Snapshot, ProviderError>>;
    fn set_credentials(&self, user: String, password: String) -> BoxFuture<'_, ()>;
    fn logout(&self) -> BoxFuture<'_, Result<(), ProviderError>>;
}

impl Backend for QnapProvider<NasClient> {
    fn snapshot(&self) -> BoxFuture<'_, Result<model::Snapshot, ProviderError>> {
        Provider::snapshot(self)
    }

    fn set_credentials(&self, user: String, password: String) -> BoxFuture<'_, ()> {
        Box::pin(self.client().set_credentials(user, password))
    }

    fn logout(&self) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(self.client().logout())
    }
}

/// Builds the backend for a NAS; sends nothing.
pub(crate) type Connector =
    Box<dyn Fn(&NasConfig, String, String) -> Result<Arc<dyn Backend>, FfiError> + Send + Sync>;

/// The real factory: QNAP over HTTPS with the pinned certificate.
pub(crate) fn connect(
    config: &NasConfig,
    user: String,
    password: String,
) -> Result<Arc<dyn Backend>, FfiError> {
    match config.provider {
        ProviderKind::Qnap => {}
        provider => return Err(FfiError::Unsupported { provider }),
    }
    let pin: Fingerprint =
        config
            .fingerprint
            .parse()
            .map_err(|e: ProviderError| FfiError::InvalidInput {
                detail: e.to_string(),
            })?;
    // The HTTP client belongs to the core runtime, also when the app calls
    // from its own thread.
    let _runtime = runtime().enter();
    let http = NasClient::new(&config.base_url, pin).map_err(|e| FfiError::InvalidInput {
        detail: e.to_string(),
    })?;
    Ok(Arc::new(QnapProvider::new(
        config.id.clone(),
        config.base_url.clone(),
        QnapClient::new(http, user, password),
    )))
}

/// A NAS of the demo data set (#22): its snapshot at the time of the clock,
/// no session to end.
struct DemoBackend {
    nas_id: String,
    clock: Clock,
}

impl Backend for DemoBackend {
    fn snapshot(&self) -> BoxFuture<'_, Result<model::Snapshot, ProviderError>> {
        let snapshot = nas_demo::demo((self.clock)())
            .snapshots
            .into_iter()
            .find(|s| s.nas.id == self.nas_id)
            .ok_or_else(|| ProviderError::Unreachable("kein Demo-NAS".into()));
        Box::pin(std::future::ready(snapshot))
    }

    fn set_credentials(&self, _user: String, _password: String) -> BoxFuture<'_, ()> {
        Box::pin(std::future::ready(()))
    }

    fn logout(&self) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The record the app sees for a state of the planner; snapshots of the
/// demo NAS are marked as demo.
fn record(state: PollState) -> NasState {
    let demo = nas_demo::is_demo_id(&state.nas_id);
    NasState {
        snapshot: state.snapshot.map(|s| {
            if demo {
                Snapshot::demo(s)
            } else {
                Snapshot::real(s)
            }
        }),
        problem: state.error.as_ref().map(NasProblem::from),
        last_attempt_at: state.last_attempt_at,
        blocked: state.blocked,
        nas_id: state.nas_id,
    }
}

struct Entry {
    backend: Arc<dyn Backend>,
    provider: model::ProviderKind,
    /// Held for a whole query, so a logout or new credentials wait for it.
    poll: tokio::sync::Mutex<()>,
    /// Goes up when the entry is removed or replaced or gets new
    /// credentials; a query started before sends nothing.
    epoch: AtomicU64,
}

impl Entry {
    /// Ends the queries started so far; they send nothing any more.
    fn retire(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// An entry as the planner sees it, at one epoch.
struct Polled {
    entry: Arc<Entry>,
    epoch: u64,
}

impl Polled {
    fn current(entry: Arc<Entry>) -> Arc<Self> {
        let epoch = entry.epoch.load(Ordering::SeqCst);
        Arc::new(Self { entry, epoch })
    }
}

impl Provider for Polled {
    fn kind(&self) -> model::ProviderKind {
        self.entry.provider
    }

    fn snapshot(&self) -> BoxFuture<'_, Result<model::Snapshot, ProviderError>> {
        Box::pin(async move {
            let _poll = self.entry.poll.lock().await;
            // Removed, replaced or new credentials while the query waited:
            // the planner discards its result, so it sends no login.
            if self.entry.epoch.load(Ordering::SeqCst) != self.epoch {
                return Err(ProviderError::Unreachable("Abfrage abgelöst".into()));
            }
            self.entry.backend.snapshot().await
        })
    }
}

type Archive = Arc<Mutex<HistoryStore>>;

/// Gets every finished query from the planner: writes the archive, then
/// calls the listener.
struct Sink {
    history: Option<Archive>,
    listener: Mutex<Option<Arc<dyn StateListener>>>,
    clock: Clock,
}

impl PollSink for Sink {
    fn polled(&self, state: &PollState) {
        if let Some(history) = &self.history {
            // An error of the archive must not stop the callback.
            let _ = archive(&mut lock(history), state, (self.clock)());
        }
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener {
            listener.state_changed(record(state.clone()));
        }
    }
}

/// Writes one finished query: after a success the snapshot and the events
/// it opens, raises or closes; after no answer (unreachable, time-out) an
/// "unreachable" event when the last snapshot is 15 minutes old. Without
/// any snapshot nothing.
fn archive(store: &mut HistoryStore, state: &PollState, now: i64) -> Result<(), HistoryError> {
    let Some(snapshot) = &state.snapshot else {
        return Ok(());
    };
    let changes = if let Some(error) = &state.error {
        // Only a NAS that gave no answer is unreachable; a refused login or
        // an odd answer still came from it.
        if !matches!(
            error,
            ProviderError::Unreachable(_) | ProviderError::Timeout
        ) {
            return Ok(());
        }
        let open = open_events(store, &state.nas_id)?;
        model::detect_unreachable(&open, &snapshot.nas, Some(snapshot.observed_at), now)
            .into_iter()
            .collect()
    } else {
        store.write_snapshot(snapshot, now)?;
        let open = open_events(store, &state.nas_id)?;
        model::detect_events(&open, snapshot, &model::Thresholds::default())
    };
    let records: Vec<EventRecord> = changes
        .iter()
        .map(|change| EventRecord::from(change.event()))
        .collect();
    store.write_events(&records)
}

/// Open events of a NAS; kinds of a newer build are left out.
fn open_events(store: &HistoryStore, nas_id: &str) -> Result<Vec<model::Event>, HistoryError> {
    Ok(store
        .open_events(nas_id)?
        .iter()
        .filter_map(|record| record.to_event().ok())
        .collect())
}

/// Fills an archive with the history, backup runs and events of the demo
/// data set.
fn fill_demo(
    store: &mut HistoryStore,
    set: &nas_demo::DemoSet,
    now: i64,
) -> Result<(), HistoryError> {
    store.write_samples(&set.samples, now)?;
    for snapshot in &set.snapshots {
        let tasks: BTreeSet<&str> = snapshot.backups.iter().map(|b| b.id.as_str()).collect();
        let runs: Vec<model::BackupRun> = set
            .backup_runs
            .iter()
            .filter(|run| tasks.contains(run.task_id.as_str()))
            .cloned()
            .collect();
        store.write_backup_runs(&snapshot.nas.id, &runs)?;
    }
    let events: Vec<EventRecord> = set.events.iter().map(EventRecord::from).collect();
    store.write_events(&events)
}

/// The NAS registered by the app, their last state and their history.
#[derive(uniffi::Object)]
pub struct NasCore {
    entries: Mutex<BTreeMap<String, Arc<Entry>>>,
    planner: Arc<Planner>,
    sink: Arc<Sink>,
    /// The task of [`Planner::run`] while the planner runs.
    run: Mutex<Option<tokio::task::JoinHandle<()>>>,
    connect: Connector,
    clock: Clock,
}

impl NasCore {
    /// Core with its own factory, clock and archive.
    pub(crate) fn with_parts(
        connect: Connector,
        clock: Clock,
        history: Option<HistoryStore>,
    ) -> Self {
        let sink = Arc::new(Sink {
            history: history.map(|store| Arc::new(Mutex::new(store))),
            listener: Mutex::new(None),
            clock: Arc::clone(&clock),
        });
        let planner = Planner::new(
            MIN_INTERVAL_SECS,
            Arc::clone(&clock),
            Arc::clone(&sink) as Arc<dyn PollSink>,
        );
        Self {
            entries: Mutex::new(BTreeMap::new()),
            planner,
            sink,
            run: Mutex::new(None),
            connect,
            clock,
        }
    }

    fn entry(&self, nas_id: &str) -> Result<Arc<Entry>, FfiError> {
        lock(&self.entries)
            .get(nas_id)
            .cloned()
            .ok_or_else(|| FfiError::UnknownNas {
                id: nas_id.to_owned(),
            })
    }

    /// Whether `entry` is still the registered entry of `nas_id`.
    fn is_current(
        entries: &BTreeMap<String, Arc<Entry>>,
        nas_id: &str,
        entry: &Arc<Entry>,
    ) -> bool {
        entries
            .get(nas_id)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
    }

    /// Registers or replaces a NAS with the planner, as never queried; the
    /// session of a replaced entry ends in the background, after a query
    /// still running on it (its error is ignored).
    fn register(&self, nas_id: String, provider: model::ProviderKind, backend: Arc<dyn Backend>) {
        let entry = Arc::new(Entry {
            backend,
            provider,
            poll: tokio::sync::Mutex::new(()),
            epoch: AtomicU64::new(0),
        });
        let mut entries = lock(&self.entries);
        let replaced = entries.insert(nas_id.clone(), Arc::clone(&entry));
        self.planner
            .add(&nas_id, Polled::current(entry), None, false);
        drop(entries);
        if let Some(old) = replaced {
            old.retire();
            runtime().spawn(async move {
                let _poll = old.poll.lock().await;
                let _ = old.backend.logout().await;
            });
        }
    }

    fn state(&self, nas_id: &str) -> Result<NasState, FfiError> {
        self.planner
            .state(nas_id)
            .map(record)
            .ok_or_else(|| FfiError::UnknownNas {
                id: nas_id.to_owned(),
            })
    }

    /// A query through the planner. With `interval` a NAS queried less
    /// than [`MIN_INTERVAL_SECS`] ago is not queried. A query refused by
    /// the planner (running, too soon, blocked) gives the current state; a
    /// NAS replaced during the query gives the state of the new entry.
    async fn query(&self, nas_id: &str, interval: bool) -> Result<NasState, FfiError> {
        let state = self
            .planner
            .state(nas_id)
            .ok_or_else(|| FfiError::UnknownNas {
                id: nas_id.to_owned(),
            })?;
        let now = (self.clock)();
        let recent = state
            .last_attempt_at
            .is_some_and(|last| now >= last && now - last < MIN_INTERVAL_SECS);
        if state.blocked || (interval && recent) {
            return Ok(record(state));
        }
        match self.planner.refresh_now(nas_id).await {
            Ok(state) => Ok(record(state)),
            Err(_) => self.state(nas_id),
        }
    }

    fn archive(&self) -> Option<MutexGuard<'_, HistoryStore>> {
        self.sink.history.as_ref().map(|history| lock(history))
    }
}

impl Default for NasCore {
    fn default() -> Self {
        Self::with_parts(
            Box::new(connect),
            system_clock(),
            HistoryStore::in_memory().ok(),
        )
    }
}

impl Drop for NasCore {
    fn drop(&mut self) {
        if let Some(run) = lock(&self.run).take() {
            run.abort();
        }
    }
}

#[uniffi::export]
impl NasCore {
    /// Core with the history archive in memory; it is gone with the core.
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self::default()
    }

    /// Core with the history archive in the SQLite file at `path`, created
    /// when missing. A file that cannot be opened is `History`.
    #[uniffi::constructor]
    pub fn with_history(path: String) -> Result<Self, FfiError> {
        let store = HistoryStore::open(Path::new(&path))?;
        Ok(Self::with_parts(
            Box::new(connect),
            system_clock(),
            Some(store),
        ))
    }

    /// Core with the three NAS of the demo data set (#22) registered and
    /// its history, backup runs and events in an archive in memory. Its
    /// clock starts at `now` (Unix seconds UTC) and runs on from there;
    /// every query answers with the demo snapshot of that time, marked as
    /// demo. Sends nothing.
    #[uniffi::constructor]
    pub fn demo(now: i64) -> Self {
        let system = system_clock();
        let offset = now.saturating_sub(system());
        let clock: Clock = Arc::new(move || system().saturating_add(offset));
        let set = nas_demo::demo(now);
        let history = HistoryStore::in_memory().ok().map(|mut store| {
            // A demo without history still shows its NAS.
            let _ = fill_demo(&mut store, &set, now);
            store
        });
        let core = Self::with_parts(Box::new(connect), Arc::clone(&clock), history);
        for snapshot in &set.snapshots {
            core.register(
                snapshot.nas.id.clone(),
                snapshot.nas.provider,
                Arc::new(DemoBackend {
                    nas_id: snapshot.nas.id.clone(),
                    clock: Arc::clone(&clock),
                }),
            );
        }
        core
    }

    /// Registers a NAS; sends nothing for a new id. An entry with the same
    /// id is replaced: its snapshot stays, the error, the time of the last
    /// query and a block are cleared, and its old session is logged out in
    /// the background.
    pub fn add_nas(
        &self,
        config: NasConfig,
        user: String,
        password: String,
    ) -> Result<(), FfiError> {
        let backend = (self.connect)(&config, user, password)?;
        self.register(config.id, config.provider.into(), backend);
        Ok(())
    }

    /// Removes a NAS, deletes its history and events and ends its session;
    /// an error of the logout or of the archive is ignored.
    pub async fn remove_nas(self: Arc<Self>, nas_id: String) -> Result<(), FfiError> {
        let entry = {
            let mut entries = lock(&self.entries);
            let entry = entries
                .remove(&nas_id)
                .ok_or(FfiError::UnknownNas { id: nas_id.clone() })?;
            // After `remove` no result of the NAS reaches the archive, so
            // nothing is written back after the delete. A new entry with
            // the same id waits for the lock and starts empty.
            self.planner.remove(&nas_id);
            entry.retire();
            if let Some(mut store) = self.archive() {
                let _ = store.delete_nas(&nas_id);
            }
            entry
        };
        on_core(async move {
            // A running query finishes first, so it does not end in a
            // login error caused by the logout.
            let _poll = entry.poll.lock().await;
            let _ = entry.backend.logout().await;
            Ok(())
        })
        .await
    }

    /// New credentials for a NAS: lifts a block, clears the error and the
    /// time of the last query, so the next [`NasCore::refresh`] queries.
    /// Waits for a query still running. Sends nothing.
    pub async fn update_credentials(
        self: Arc<Self>,
        nas_id: String,
        user: String,
        password: String,
    ) -> Result<(), FfiError> {
        on_core(async move {
            // The entry may be replaced or removed while the call waits for
            // a running query; the credentials go to the entry registered
            // after the wait.
            loop {
                let entry = self.entry(&nas_id)?;
                let _poll = entry.poll.lock().await;
                if !Self::is_current(&lock(&self.entries), &nas_id, &entry) {
                    continue;
                }
                entry.backend.set_credentials(user, password).await;
                // A query that waited for the lock would send the new
                // credentials a second time right after the next one.
                entry.retire();
                let entries = lock(&self.entries);
                // Replaced meanwhile: the new entry starts fresh anyway.
                if Self::is_current(&entries, &nas_id, &entry) {
                    self.planner
                        .add(&nas_id, Polled::current(Arc::clone(&entry)), None, false);
                }
                return Ok(());
            }
        })
        .await
    }

    /// Queries the NAS, unless it is blocked, was queried less than
    /// [`MIN_INTERVAL_SECS`] ago or a query is running already; then
    /// returns the current state without a request and without waiting. A
    /// failed query does not throw: its error is in [`NasState::problem`],
    /// the last snapshot stays.
    pub async fn refresh(self: Arc<Self>, nas_id: String) -> Result<NasState, FfiError> {
        on_core(async move { self.query(&nas_id, true).await }).await
    }

    /// Manual refresh: like [`NasCore::refresh`], but already 30 seconds
    /// after the last query. The next automatic query follows one interval
    /// later.
    pub async fn refresh_now(self: Arc<Self>, nas_id: String) -> Result<NasState, FfiError> {
        on_core(async move { self.query(&nas_id, false).await }).await
    }

    /// Hands back what the app remembers across launches: the start of the
    /// last query and whether the NAS is blocked. Call it after
    /// [`NasCore::add_nas`] and before [`NasCore::start_planner`], so a
    /// restart neither queries at once nor repeats a rejected login. Clears
    /// the error; the snapshot stays. Sends nothing.
    pub fn restore_poll(
        &self,
        nas_id: String,
        last_attempt_at: Option<i64>,
        blocked: bool,
    ) -> Result<(), FfiError> {
        let entries = lock(&self.entries);
        let entry = entries
            .get(&nas_id)
            .cloned()
            .ok_or_else(|| FfiError::UnknownNas { id: nas_id.clone() })?;
        self.planner
            .add(&nas_id, Polled::current(entry), last_attempt_at, blocked);
        Ok(())
    }

    /// State of every registered NAS, sorted by id; sends nothing.
    pub fn states(&self) -> Vec<NasState> {
        self.planner.states().into_iter().map(record).collect()
    }

    /// Starts the planner: from now on the core queries every NAS when it
    /// is due and calls `listener` after every finished query, also a
    /// failed one, never after a skipped one. A running planner only gets
    /// the new listener. See [`StateListener`] for what the listener must
    /// not do.
    pub fn start_planner(&self, listener: Arc<dyn StateListener>) {
        *lock(&self.sink.listener) = Some(listener);
        let mut run = lock(&self.run);
        if run.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        *run = Some(runtime().spawn(Arc::clone(&self.planner).run()));
    }

    /// Stops the planner and drops the listener. A query in flight still
    /// ends and is archived; a callback already under way may still reach
    /// the old listener, later ones do not. [`NasCore::start_planner`]
    /// starts it again.
    pub fn stop_planner(&self) {
        *lock(&self.sink.listener) = None;
        if let Some(run) = lock(&self.run).take() {
            run.abort();
        }
    }

    /// Interval between automatic queries of the planner, in seconds; less
    /// than [`MIN_INTERVAL_SECS`] counts as the minimum.
    pub fn set_interval(&self, secs: i64) {
        self.planner.set_interval(secs);
    }

    /// All series of a metric of a NAS in a range ending now, in the
    /// resolution of the range, sorted by key with `None` first. Empty for
    /// an unknown NAS or metric without values.
    pub fn history(
        &self,
        nas_id: String,
        metric: Metric,
        range: HistoryRange,
    ) -> Result<Vec<HistorySeries>, FfiError> {
        let now = (self.clock)();
        let Some(store) = self.archive() else {
            return Ok(Vec::new());
        };
        Ok(store
            .history(&nas_id, metric.into(), range.into(), now)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Events of one NAS (`None`: of all NAS) that were open at some time
    /// in `[from, to]` (Unix seconds UTC), newest first.
    pub fn events(
        &self,
        nas_id: Option<String>,
        from: i64,
        to: i64,
    ) -> Result<Vec<NasEvent>, FfiError> {
        let Some(store) = self.archive() else {
            return Ok(Vec::new());
        };
        Ok(store
            .events(nas_id.as_deref(), from, to)?
            .iter()
            // Kinds of a newer build are left out, as for the detection.
            .filter_map(|record| record.to_event().ok())
            .map(NasEvent::from)
            .collect())
    }

    /// Forecast "full in X days" of a volume from its daily values of the
    /// last 30 days; `None` without a rising trend, for an unknown NAS or
    /// volume, or without a known size.
    pub fn forecast(
        &self,
        nas_id: String,
        volume_id: String,
    ) -> Result<Option<Forecast>, FfiError> {
        let now = (self.clock)();
        let Some(store) = self.archive() else {
            return Ok(None);
        };
        Ok(store.forecast(&nas_id, &volume_id, now)?.map(Into::into))
    }

    /// Backup runs of a NAS started in `[from, to]` (Unix seconds UTC),
    /// oldest first.
    pub fn backup_runs(
        &self,
        nas_id: String,
        from: i64,
        to: i64,
    ) -> Result<Vec<BackupRun>, FfiError> {
        let Some(store) = self.archive() else {
            return Ok(Vec::new());
        };
        Ok(store
            .backup_runs(&nas_id, from, to)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The last snapshot read from the NAS as JSON, for the app to save;
    /// `None` before the first successful query. Sends nothing.
    pub fn saved_state(&self, nas_id: String) -> Result<Option<String>, FfiError> {
        self.entry(&nas_id)?;
        self.planner
            .state(&nas_id)
            .and_then(|state| state.snapshot)
            .map(|raw| {
                serde_json::to_string(&raw).map_err(|e| FfiError::Internal {
                    detail: e.to_string(),
                })
            })
            .transpose()
    }

    /// Hands back a snapshot saved with [`NasCore::saved_state`], e.g.
    /// after a restart of the app. A NAS that already has a snapshot keeps
    /// it. The error, the time of the last query and a block stay as they
    /// are, so the next [`NasCore::refresh`] queries as before. Sends
    /// nothing. JSON that does not parse or belongs to another NAS is
    /// `InvalidInput`.
    pub fn restore_state(&self, nas_id: String, json: String) -> Result<(), FfiError> {
        self.entry(&nas_id)?;
        let raw: model::Snapshot =
            serde_json::from_str(&json).map_err(|e| FfiError::InvalidInput {
                detail: e.to_string(),
            })?;
        if raw.nas.id != nas_id {
            return Err(FfiError::InvalidInput {
                detail: format!("saved state of {} given for {nas_id}", raw.nas.id),
            });
        }
        self.planner.restore_snapshot(&nas_id, raw);
        Ok(())
    }

    /// Queries a NAS once without registering it, e.g. before saving it,
    /// and logs out again (an error of the logout is ignored). Ignores the
    /// interval; an error is thrown. Nothing goes into the archive.
    pub async fn test_connection(
        self: Arc<Self>,
        config: NasConfig,
        user: String,
        password: String,
    ) -> Result<Snapshot, FfiError> {
        on_core(async move {
            let backend = (self.connect)(&config, user, password)?;
            let result = backend.snapshot().await;
            let _ = backend.logout().await;
            Ok(Snapshot::real(result?))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    const T0: i64 = 1_800_000_000;

    fn model_snapshot(cpu: f64) -> model::Snapshot {
        model::Snapshot {
            nas: model::Nas {
                id: "nas-1".into(),
                provider: model::ProviderKind::Qnap,
                name: None,
                base_url: "https://nas.example".into(),
                model: Some("TS-000".into()),
                firmware: None,
                hostname: None,
            },
            observed_at: T0,
            system: model::SystemStatus {
                cpu_percent: Some(cpu),
                ..model::SystemStatus::default()
            },
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

    #[derive(Default)]
    struct FakeBackend {
        answers: Mutex<VecDeque<Result<model::Snapshot, ProviderError>>>,
        queries: AtomicUsize,
        logouts: AtomicUsize,
        credentials: Mutex<Vec<(String, String)>>,
        /// Delay of every query, so parallel calls overlap.
        slow: bool,
        logout_fails: bool,
        /// When set, a query signals `started` and waits for `release`.
        gate: Option<Arc<Gate>>,
    }

    #[derive(Default)]
    struct Gate {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    impl FakeBackend {
        fn answering(answers: Vec<Result<model::Snapshot, ProviderError>>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.into()),
                ..Self::default()
            })
        }

        fn queries(&self) -> usize {
            self.queries.load(Ordering::SeqCst)
        }
    }

    impl Backend for FakeBackend {
        fn snapshot(&self) -> BoxFuture<'_, Result<model::Snapshot, ProviderError>> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            let answer = lock(&self.answers)
                .pop_front()
                .unwrap_or_else(|| Err(ProviderError::Parse("no answer left".into())));
            let slow = self.slow;
            let gate = self.gate.clone();
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.started.notify_one();
                    gate.release.notified().await;
                }
                if slow {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                answer
            })
        }

        fn set_credentials(&self, user: String, password: String) -> BoxFuture<'_, ()> {
            lock(&self.credentials).push((user, password));
            Box::pin(std::future::ready(()))
        }

        fn logout(&self) -> BoxFuture<'_, Result<(), ProviderError>> {
            self.logouts.fetch_add(1, Ordering::SeqCst);
            let result = if self.logout_fails {
                Err(ProviderError::Timeout)
            } else {
                Ok(())
            };
            Box::pin(std::future::ready(result))
        }
    }

    struct Harness {
        core: Arc<NasCore>,
        clock: Arc<AtomicI64>,
        connected: Arc<Mutex<Vec<(NasConfig, String, String)>>>,
    }

    impl Harness {
        fn set_time(&self, at: i64) {
            self.clock.store(at, Ordering::SeqCst);
        }
    }

    fn harness(backend: Arc<FakeBackend>) -> Harness {
        harness_with(vec![backend])
    }

    /// Each connect hands out the next backend; the last one is reused.
    fn harness_with(backends: Vec<Arc<FakeBackend>>) -> Harness {
        let clock = Arc::new(AtomicI64::new(T0));
        let connected = Arc::new(Mutex::new(Vec::new()));
        let backends = Mutex::new(VecDeque::from(backends));
        let core = {
            let clock = clock.clone();
            let connected = connected.clone();
            NasCore::with_parts(
                Box::new(move |config: &NasConfig, user, password| {
                    lock(&connected).push((config.clone(), user, password));
                    let mut queue = lock(&backends);
                    let backend = if queue.len() > 1 {
                        queue.pop_front()
                    } else {
                        queue.front().cloned()
                    };
                    let backend = backend.ok_or(FfiError::Internal {
                        detail: "no backend".into(),
                    })?;
                    Ok(backend as Arc<dyn Backend>)
                }),
                Arc::new(move || clock.load(Ordering::SeqCst)),
                HistoryStore::in_memory().ok(),
            )
        };
        Harness {
            core: Arc::new(core),
            clock,
            connected,
        }
    }

    fn config(id: &str) -> NasConfig {
        NasConfig {
            id: id.into(),
            provider: ProviderKind::Qnap,
            base_url: "https://nas.example".into(),
            fingerprint: "AB".repeat(32),
        }
    }

    fn cpu(state: &NasState) -> Option<f64> {
        state.snapshot.as_ref().and_then(|s| s.system.cpu_percent)
    }

    #[tokio::test]
    async fn add_sends_nothing_and_refresh_queries() {
        let backend = FakeBackend::answering(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "monitor".into(), "secret".into())
            .expect("add");
        assert_eq!(backend.queries(), 0);
        let connected = lock(&h.connected).clone();
        assert_eq!(
            connected,
            vec![(config("nas-1"), "monitor".into(), "secret".into())]
        );
        assert_eq!(
            h.core.states(),
            vec![NasState {
                nas_id: "nas-1".into(),
                snapshot: None,
                problem: None,
                last_attempt_at: None,
                blocked: false,
            }]
        );

        let state = h.core.clone().refresh("nas-1".into()).await.expect("state");
        assert_eq!(backend.queries(), 1);
        assert_eq!(cpu(&state), Some(10.0));
        assert_eq!(state.problem, None);
        assert_eq!(state.last_attempt_at, Some(T0));
        assert!(!state.blocked);
        assert!(!state.snapshot.as_ref().expect("snapshot").demo);
        assert_eq!(h.core.states(), vec![state]);
    }

    #[tokio::test]
    async fn refresh_within_five_minutes_sends_nothing() {
        let backend =
            FakeBackend::answering(vec![Ok(model_snapshot(10.0)), Ok(model_snapshot(20.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");

        h.set_time(T0 + MIN_INTERVAL_SECS - 1);
        let state = h
            .core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("cached");
        assert_eq!(backend.queries(), 1);
        assert_eq!(cpu(&state), Some(10.0));
        assert_eq!(state.last_attempt_at, Some(T0));

        h.set_time(T0 + MIN_INTERVAL_SECS);
        let state = h.core.clone().refresh("nas-1".into()).await.expect("due");
        assert_eq!(backend.queries(), 2);
        assert_eq!(cpu(&state), Some(20.0));
        assert_eq!(state.last_attempt_at, Some(T0 + MIN_INTERVAL_SECS));
    }

    #[tokio::test]
    async fn a_clock_set_back_counts_as_due() {
        let backend =
            FakeBackend::answering(vec![Ok(model_snapshot(10.0)), Ok(model_snapshot(20.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");
        h.set_time(T0 - 60);
        let state = h.core.clone().refresh("nas-1".into()).await.expect("due");
        assert_eq!(backend.queries(), 2);
        assert_eq!(cpu(&state), Some(20.0));
    }

    #[tokio::test]
    async fn an_error_keeps_the_last_snapshot_and_polling_goes_on() {
        let backend = FakeBackend::answering(vec![
            Ok(model_snapshot(10.0)),
            Err(ProviderError::Timeout),
            Ok(model_snapshot(30.0)),
        ]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");

        h.set_time(T0 + MIN_INTERVAL_SECS);
        let state = h
            .core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("failed");
        assert_eq!(cpu(&state), Some(10.0));
        let problem = state.problem.expect("problem");
        assert_eq!(problem.kind, crate::records::ProblemKind::Timeout);
        assert!(!problem.stops_polling);
        assert!(!state.blocked);

        h.set_time(T0 + 2 * MIN_INTERVAL_SECS);
        let state = h.core.clone().refresh("nas-1".into()).await.expect("again");
        assert_eq!(backend.queries(), 3);
        assert_eq!(cpu(&state), Some(30.0));
        assert_eq!(state.problem, None);
    }

    #[tokio::test]
    async fn a_rejected_login_blocks_until_new_credentials() {
        let backend = FakeBackend::answering(vec![
            Ok(model_snapshot(10.0)),
            Err(ProviderError::LoginFailed),
            Ok(model_snapshot(40.0)),
        ]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");
        h.set_time(T0 + MIN_INTERVAL_SECS);
        let state = h
            .core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("refused");
        assert!(state.blocked);
        assert_eq!(cpu(&state), Some(10.0));
        assert!(state.problem.as_ref().expect("problem").stops_polling);

        // Blocked: no query, however much time passes.
        h.set_time(T0 + 100 * MIN_INTERVAL_SECS);
        let state = h
            .core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("blocked");
        assert_eq!(backend.queries(), 2);
        assert!(state.blocked);

        h.core
            .clone()
            .update_credentials("nas-1".into(), "monitor".into(), "new".into())
            .await
            .expect("update");
        assert_eq!(
            lock(&backend.credentials).clone(),
            vec![("monitor".to_owned(), "new".to_owned())]
        );
        let state = &h.core.states()[0];
        assert!(!state.blocked);
        assert_eq!(state.problem, None);
        assert_eq!(state.last_attempt_at, None);
        assert_eq!(cpu(state), Some(10.0));

        let state = h.core.clone().refresh("nas-1".into()).await.expect("again");
        assert_eq!(backend.queries(), 3);
        assert_eq!(cpu(&state), Some(40.0));
    }

    #[tokio::test]
    async fn second_factor_and_certificate_change_block_too() {
        for error in [
            ProviderError::SecondFactorRequired,
            ProviderError::CertificateChanged {
                old: "AA".into(),
                new: "BB".into(),
            },
        ] {
            let backend = FakeBackend::answering(vec![Err(error)]);
            let h = harness(backend.clone());
            h.core
                .add_nas(config("nas-1"), "u".into(), "p".into())
                .expect("add");
            let state = h.core.clone().refresh("nas-1".into()).await.expect("state");
            assert!(state.blocked);
            assert_eq!(state.snapshot, None);
            h.set_time(T0 + MIN_INTERVAL_SECS);
            h.core.clone().refresh("nas-1".into()).await.expect("state");
            assert_eq!(backend.queries(), 1);
        }
    }

    #[tokio::test]
    async fn adding_again_keeps_the_snapshot_and_lifts_the_block() {
        let backend = FakeBackend::answering(vec![
            Ok(model_snapshot(10.0)),
            Err(ProviderError::LoginFailed),
            Ok(model_snapshot(50.0)),
        ]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");
        h.set_time(T0 + MIN_INTERVAL_SECS);
        h.core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("refused");

        h.core
            .add_nas(config("nas-1"), "u".into(), "p2".into())
            .expect("add again");
        let states = h.core.states();
        assert_eq!(states.len(), 1);
        assert!(!states[0].blocked);
        assert_eq!(states[0].problem, None);
        assert_eq!(states[0].last_attempt_at, None);
        assert_eq!(cpu(&states[0]), Some(10.0));

        let state = h.core.clone().refresh("nas-1".into()).await.expect("again");
        assert_eq!(backend.queries(), 3);
        assert_eq!(cpu(&state), Some(50.0));
    }

    #[tokio::test]
    async fn a_parallel_refresh_does_not_wait_for_the_running_query() {
        let (backend, gate) = gated(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let running = tokio::spawn(h.core.clone().refresh("nas-1".into()));
        gate.started.notified().await;
        // The second call gets the current state at once, without a query.
        let second = h.core.clone().refresh("nas-1".into()).await.expect("b");
        assert_eq!(cpu(&second), None);
        assert_eq!(second.last_attempt_at, Some(T0));
        gate.release.notify_one();
        let first = running.await.expect("join").expect("a");
        assert_eq!(cpu(&first), Some(10.0));
        assert_eq!(backend.queries(), 1);
    }

    #[tokio::test]
    async fn remove_logs_out_and_forgets_the_nas() {
        let backend = Arc::new(FakeBackend {
            logout_fails: true,
            ..FakeBackend::default()
        });
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core
            .clone()
            .remove_nas("nas-1".into())
            .await
            .expect("logout error is ignored");
        assert_eq!(backend.logouts.load(Ordering::SeqCst), 1);
        assert!(h.core.states().is_empty());
        assert_eq!(
            h.core.clone().remove_nas("nas-1".into()).await,
            Err(FfiError::UnknownNas { id: "nas-1".into() })
        );
    }

    fn gated(
        answers: Vec<Result<model::Snapshot, ProviderError>>,
    ) -> (Arc<FakeBackend>, Arc<Gate>) {
        let gate = Arc::new(Gate::default());
        let backend = Arc::new(FakeBackend {
            answers: Mutex::new(answers.into()),
            gate: Some(gate.clone()),
            ..FakeBackend::default()
        });
        (backend, gate)
    }

    #[tokio::test]
    async fn removing_during_a_query_waits_and_reports_unknown() {
        let (backend, gate) = gated(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let running = tokio::spawn(h.core.clone().refresh("nas-1".into()));
        gate.started.notified().await;
        let removing = tokio::spawn(h.core.clone().remove_nas("nas-1".into()));
        // The NAS is gone at once, the logout waits for the query.
        while !h.core.states().is_empty() {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(backend.logouts.load(Ordering::SeqCst), 0);
        gate.release.notify_one();
        assert_eq!(
            running.await.expect("join"),
            Err(FfiError::UnknownNas { id: "nas-1".into() })
        );
        removing.await.expect("join").expect("remove");
        assert_eq!(backend.queries(), 1);
        assert_eq!(backend.logouts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn replacing_during_a_query_drops_its_result_and_logs_out_after_it() {
        let (backend, gate) = gated(vec![Err(ProviderError::LoginFailed)]);
        let fresh = FakeBackend::answering(Vec::new());
        let h = harness_with(vec![backend.clone(), fresh.clone()]);
        h.core
            .add_nas(config("nas-1"), "u".into(), "wrong".into())
            .expect("add");
        let running = tokio::spawn(h.core.clone().refresh("nas-1".into()));
        gate.started.notified().await;
        h.core
            .add_nas(config("nas-1"), "u".into(), "right".into())
            .expect("replace");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(backend.logouts.load(Ordering::SeqCst), 0);
        gate.release.notify_one();

        // The login error of the old entry does not reach the new one.
        let state = running.await.expect("join").expect("state");
        assert!(!state.blocked);
        assert_eq!(state.problem, None);
        assert_eq!(h.core.states(), vec![state]);
        for _ in 0..100 {
            if backend.logouts.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(backend.logouts.load(Ordering::SeqCst), 1);
        // Only the old session ends; the new entry stays untouched.
        assert_eq!(fresh.logouts.load(Ordering::SeqCst), 0);
        assert_eq!(fresh.queries(), 0);
    }

    #[tokio::test]
    async fn new_credentials_after_a_replace_reach_the_registered_entry() {
        let (old, gate) = gated(vec![Err(ProviderError::LoginFailed)]);
        let fresh = FakeBackend::answering(Vec::new());
        let h = harness_with(vec![old.clone(), fresh.clone()]);
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let running = tokio::spawn(h.core.clone().refresh("nas-1".into()));
        gate.started.notified().await;
        let updating = tokio::spawn(h.core.clone().update_credentials(
            "nas-1".into(),
            "monitor".into(),
            "new".into(),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;
        h.core
            .add_nas(config("nas-1"), "u".into(), "p2".into())
            .expect("replace");
        gate.release.notify_one();
        running.await.expect("join").expect("state");
        updating.await.expect("join").expect("update");
        assert!(lock(&old.credentials).is_empty());
        assert_eq!(
            lock(&fresh.credentials).clone(),
            vec![("monitor".to_owned(), "new".to_owned())]
        );
    }

    #[tokio::test]
    async fn new_credentials_for_a_nas_removed_meanwhile_fail() {
        let (backend, gate) = gated(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let running = tokio::spawn(h.core.clone().refresh("nas-1".into()));
        gate.started.notified().await;
        let updating = tokio::spawn(h.core.clone().update_credentials(
            "nas-1".into(),
            "monitor".into(),
            "new".into(),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let removing = tokio::spawn(h.core.clone().remove_nas("nas-1".into()));
        while !h.core.states().is_empty() {
            tokio::task::yield_now().await;
        }
        gate.release.notify_one();
        let _ = running.await.expect("join");
        assert_eq!(
            updating.await.expect("join"),
            Err(FfiError::UnknownNas { id: "nas-1".into() })
        );
        removing.await.expect("join").expect("remove");
        assert!(lock(&backend.credentials).is_empty());
    }

    #[tokio::test]
    async fn unknown_nas_is_an_error() {
        let h = harness(FakeBackend::answering(Vec::new()));
        assert_eq!(
            h.core.clone().refresh("nope".into()).await,
            Err(FfiError::UnknownNas { id: "nope".into() })
        );
        assert_eq!(
            h.core
                .clone()
                .update_credentials("nope".into(), "u".into(), "p".into())
                .await,
            Err(FfiError::UnknownNas { id: "nope".into() })
        );
    }

    #[tokio::test]
    async fn a_saved_state_comes_back_in_a_new_core() {
        let backend = FakeBackend::answering(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        assert_eq!(h.core.saved_state("nas-1".into()).expect("saved"), None);
        let state = h.core.clone().refresh("nas-1".into()).await.expect("state");
        let json = h
            .core
            .saved_state("nas-1".into())
            .expect("saved")
            .expect("json");

        let restarted = harness(FakeBackend::answering(Vec::new()));
        restarted
            .core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        restarted
            .core
            .restore_state("nas-1".into(), json.clone())
            .expect("restore");
        let restored = restarted.core.states().remove(0);
        assert_eq!(restored.snapshot, state.snapshot);
        assert_eq!(restored.problem, None);
        assert_eq!(restored.last_attempt_at, None);
        assert!(!restored.blocked);
        assert_eq!(
            restarted.core.saved_state("nas-1".into()).expect("saved"),
            Some(json)
        );
    }

    #[tokio::test]
    async fn restoring_refuses_other_nas_and_broken_json() {
        let backend = FakeBackend::answering(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core
            .add_nas(config("nas-2"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("state");
        let json = h
            .core
            .saved_state("nas-1".into())
            .expect("saved")
            .expect("json");

        assert!(matches!(
            h.core.restore_state("nas-2".into(), json.clone()),
            Err(FfiError::InvalidInput { .. })
        ));
        assert!(matches!(
            h.core.restore_state("nas-2".into(), "{\"nas\":".into()),
            Err(FfiError::InvalidInput { .. })
        ));
        assert!(matches!(
            h.core.restore_state("nas-9".into(), json),
            Err(FfiError::UnknownNas { .. })
        ));
        assert!(matches!(
            h.core.saved_state("nas-9".into()),
            Err(FfiError::UnknownNas { .. })
        ));
        assert_eq!(h.core.states()[1].snapshot, None);
        assert_eq!(backend.queries(), 1);
    }

    #[tokio::test]
    async fn restoring_keeps_a_newer_snapshot() {
        let backend =
            FakeBackend::answering(vec![Ok(model_snapshot(10.0)), Ok(model_snapshot(20.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");
        let old = h
            .core
            .saved_state("nas-1".into())
            .expect("saved")
            .expect("json");
        h.set_time(T0 + MIN_INTERVAL_SECS);
        h.core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("second");

        h.core
            .restore_state("nas-1".into(), old.clone())
            .expect("restore");
        let state = h.core.states().remove(0);
        assert_eq!(cpu(&state), Some(20.0));
        assert_eq!(state.last_attempt_at, Some(T0 + MIN_INTERVAL_SECS));
        assert_ne!(
            h.core.saved_state("nas-1".into()).expect("saved"),
            Some(old)
        );
        assert_eq!(backend.queries(), 2);
    }

    #[tokio::test]
    async fn states_are_sorted_by_id() {
        let h = harness(FakeBackend::answering(Vec::new()));
        assert!(h.core.states().is_empty());
        for id in ["nas-b", "nas-a", "nas-c"] {
            h.core
                .add_nas(config(id), "u".into(), "p".into())
                .expect("add");
        }
        let ids: Vec<_> = h.core.states().into_iter().map(|s| s.nas_id).collect();
        assert_eq!(ids, ["nas-a", "nas-b", "nas-c"]);
    }

    #[tokio::test]
    async fn test_connection_queries_once_and_logs_out() {
        let backend = FakeBackend::answering(vec![
            Ok(model_snapshot(10.0)),
            Ok(model_snapshot(20.0)),
            Err(ProviderError::LoginFailed),
        ]);
        let h = harness(backend.clone());
        let snapshot = h
            .core
            .clone()
            .test_connection(config("new"), "u".into(), "p".into())
            .await
            .expect("snapshot");
        assert_eq!(snapshot.system.cpu_percent, Some(10.0));
        assert!(!snapshot.demo);
        // No interval and no entry.
        h.core
            .clone()
            .test_connection(config("new"), "u".into(), "p".into())
            .await
            .expect("again");
        assert!(h.core.states().is_empty());
        assert_eq!(
            h.core
                .clone()
                .test_connection(config("new"), "u".into(), "bad".into())
                .await,
            Err(FfiError::LoginFailed)
        );
        assert_eq!(backend.queries(), 3);
        assert_eq!(backend.logouts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn factory_refuses_other_vendors_http_and_bad_fingerprints() {
        let synology = NasConfig {
            provider: ProviderKind::Synology,
            ..config("nas-1")
        };
        assert!(matches!(
            connect(&synology, "u".into(), "p".into()),
            Err(FfiError::Unsupported {
                provider: ProviderKind::Synology
            })
        ));

        let http = NasConfig {
            base_url: "http://nas.example".into(),
            ..config("nas-1")
        };
        assert!(matches!(
            connect(&http, "u".into(), "p".into()),
            Err(FfiError::InvalidInput { .. })
        ));

        let bad_pin = NasConfig {
            fingerprint: "nope".into(),
            ..config("nas-1")
        };
        assert!(matches!(
            connect(&bad_pin, "u".into(), "p".into()),
            Err(FfiError::InvalidInput { .. })
        ));

        assert!(connect(&config("nas-1"), "u".into(), "p".into()).is_ok());
    }

    #[test]
    fn the_real_core_adds_a_qnap_without_network() {
        let core = NasCore::new();
        core.add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        assert_eq!(core.states().len(), 1);
        let http = NasConfig {
            base_url: "http://nas.example".into(),
            ..config("nas-2")
        };
        assert!(matches!(
            core.add_nas(http, "u".into(), "p".into()),
            Err(FfiError::InvalidInput { .. })
        ));
        assert_eq!(core.states().len(), 1);
    }

    /// A snapshot with disk 1 at `temp` °C.
    fn hot_snapshot(temp: f64, observed_at: i64) -> model::Snapshot {
        model::Snapshot {
            observed_at,
            disks: vec![model::Disk {
                id: "disk-1".into(),
                name: "Platte 1".into(),
                model: None,
                serial_hash: None,
                temperature_c: Some(temp),
                health: Some(model::Health::Ok),
                smart_text: None,
                power_on_hours: None,
                spinning: None,
            }],
            ..model_snapshot(10.0)
        }
    }

    #[tokio::test]
    async fn every_query_goes_into_history_and_events() {
        let backend = FakeBackend::answering(vec![
            Ok(hot_snapshot(55.0, T0)),
            Ok(hot_snapshot(40.0, T0 + MIN_INTERVAL_SECS)),
        ]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");

        let series = h
            .core
            .history("nas-1".into(), Metric::DiskTemp, HistoryRange::Day)
            .expect("history");
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].key.as_deref(), Some("disk-1"));
        assert_eq!(series[0].points.len(), 1);
        assert_eq!(series[0].points[0].at, T0);
        assert_eq!(series[0].points[0].mean, 55.0);
        let events = h.core.events(Some("nas-1".into()), T0, T0).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, crate::records::EventKind::DiskTemperature);
        assert_eq!(events[0].subject.as_deref(), Some("disk-1"));
        assert_eq!(events[0].level, crate::records::Health::Warning);
        assert_eq!(events[0].started_at, T0);
        assert_eq!(events[0].ended_at, None);

        // Cooled down: the event ends with the next snapshot.
        h.set_time(T0 + MIN_INTERVAL_SECS);
        h.core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("second");
        let events = h.core.events(None, T0, T0 + DAY).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ended_at, Some(T0 + MIN_INTERVAL_SECS));
        let series = h
            .core
            .history("nas-1".into(), Metric::DiskTemp, HistoryRange::Day)
            .expect("history");
        assert_eq!(series[0].points.len(), 2);
    }

    const DAY: i64 = 86_400;

    #[tokio::test]
    async fn an_error_15_minutes_after_the_last_snapshot_opens_unreachable() {
        let backend = FakeBackend::answering(vec![
            Ok(model_snapshot(10.0)),
            Err(ProviderError::Timeout),
            Err(ProviderError::Timeout),
            Err(ProviderError::Timeout),
        ]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");

        // 5 minutes: not yet unreachable.
        h.set_time(T0 + MIN_INTERVAL_SECS);
        h.core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("failed");
        assert!(
            h.core
                .events(None, T0, T0 + DAY)
                .expect("events")
                .is_empty()
        );

        // 15 minutes after the snapshot: opened once, from the last snapshot.
        for step in [3, 4] {
            h.set_time(T0 + step * MIN_INTERVAL_SECS);
            h.core
                .clone()
                .refresh("nas-1".into())
                .await
                .expect("failed");
        }
        let events = h.core.events(None, T0, T0 + DAY).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, crate::records::EventKind::Unreachable);
        assert_eq!(events[0].level, crate::records::Health::Critical);
        assert_eq!(events[0].started_at, T0);
        assert_eq!(events[0].ended_at, None);
        assert_eq!(backend.queries(), 4);
    }

    #[tokio::test]
    async fn an_error_without_any_snapshot_writes_nothing() {
        let backend = FakeBackend::answering(vec![Err(ProviderError::Timeout)]);
        let h = harness(backend);
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.set_time(T0 + 100 * MIN_INTERVAL_SECS);
        h.core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("failed");
        assert!(h.core.events(None, 0, i64::MAX).expect("events").is_empty());
    }

    /// Hands every state to a channel.
    struct Listener(Mutex<std::sync::mpsc::Sender<NasState>>);

    impl StateListener for Listener {
        fn state_changed(&self, state: NasState) {
            let _ = lock(&self.0).send(state);
        }
    }

    fn listener() -> (Arc<Listener>, std::sync::mpsc::Receiver<NasState>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Arc::new(Listener(Mutex::new(tx))), rx)
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn the_planner_queries_and_calls_back_without_refresh() {
        let backend =
            FakeBackend::answering(vec![Ok(model_snapshot(10.0)), Err(ProviderError::Timeout)]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let (listener, states) = listener();
        h.core.start_planner(listener);
        let state = states.recv_timeout(WAIT).expect("callback");
        assert_eq!(state.nas_id, "nas-1");
        assert_eq!(cpu(&state), Some(10.0));
        assert_eq!(state.last_attempt_at, Some(T0));
        assert_eq!(backend.queries(), 1);
        // The archive has it too.
        assert_eq!(
            h.core
                .history("nas-1".into(), Metric::Cpu, HistoryRange::Day)
                .expect("history")[0]
                .points
                .len(),
            1
        );

        // A failed query calls back as well, with the snapshot kept.
        h.set_time(T0 + MIN_INTERVAL_SECS);
        h.core.set_interval(MIN_INTERVAL_SECS);
        let state = states.recv_timeout(WAIT).expect("callback after error");
        assert_eq!(cpu(&state), Some(10.0));
        assert_eq!(
            state.problem.expect("problem").kind,
            crate::records::ProblemKind::Timeout
        );
        h.core.stop_planner();
    }

    #[test]
    fn a_stopped_planner_queries_no_more_and_starts_again() {
        let backend =
            FakeBackend::answering(vec![Ok(model_snapshot(10.0)), Ok(model_snapshot(20.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let (first, states) = listener();
        h.core.start_planner(first);
        assert_eq!(
            cpu(&states.recv_timeout(WAIT).expect("callback")),
            Some(10.0)
        );
        h.core.stop_planner();

        // Due again, but stopped: no query.
        h.set_time(T0 + MIN_INTERVAL_SECS);
        h.core.set_interval(MIN_INTERVAL_SECS);
        assert!(states.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(backend.queries(), 1);

        let (second, states) = listener();
        h.core.start_planner(second);
        assert_eq!(
            cpu(&states.recv_timeout(WAIT).expect("callback")),
            Some(20.0)
        );
        assert_eq!(backend.queries(), 2);
        h.core.stop_planner();
    }

    #[test]
    fn a_restored_block_is_never_queried() {
        let backend = FakeBackend::answering(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core
            .restore_poll("nas-1".into(), Some(T0 - 3_600), true)
            .expect("restore");
        let state = h.core.states().remove(0);
        assert!(state.blocked);
        assert_eq!(state.last_attempt_at, Some(T0 - 3_600));

        let (listener, states) = listener();
        h.core.start_planner(listener);
        assert!(states.recv_timeout(Duration::from_millis(200)).is_err());
        h.core.stop_planner();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let state = rt
            .block_on(h.core.clone().refresh_now("nas-1".into()))
            .expect("state");
        assert!(state.blocked);
        assert_eq!(backend.queries(), 0);
        assert!(matches!(
            h.core.restore_poll("nas-9".into(), None, false),
            Err(FfiError::UnknownNas { .. })
        ));
    }

    #[tokio::test]
    async fn a_restored_time_waits_for_the_interval() {
        let backend = FakeBackend::answering(vec![Ok(model_snapshot(10.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core
            .restore_poll("nas-1".into(), Some(T0 - 60), false)
            .expect("restore");
        h.core.clone().refresh("nas-1".into()).await.expect("state");
        assert_eq!(backend.queries(), 0);
        // The manual refresh only waits 30 seconds.
        let state = h
            .core
            .clone()
            .refresh_now("nas-1".into())
            .await
            .expect("state");
        assert_eq!(backend.queries(), 1);
        assert_eq!(cpu(&state), Some(10.0));
    }

    #[tokio::test]
    async fn refresh_now_waits_30_seconds() {
        let backend =
            FakeBackend::answering(vec![Ok(model_snapshot(10.0)), Ok(model_snapshot(20.0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core
            .clone()
            .refresh_now("nas-1".into())
            .await
            .expect("first");
        h.set_time(T0 + 29);
        let state = h
            .core
            .clone()
            .refresh_now("nas-1".into())
            .await
            .expect("too soon");
        assert_eq!(cpu(&state), Some(10.0));
        assert_eq!(backend.queries(), 1);
        h.set_time(T0 + 30);
        let state = h
            .core
            .clone()
            .refresh_now("nas-1".into())
            .await
            .expect("again");
        assert_eq!(cpu(&state), Some(20.0));
        assert_eq!(
            h.core.clone().refresh_now("nope".into()).await,
            Err(FfiError::UnknownNas { id: "nope".into() })
        );
    }

    #[tokio::test]
    async fn removing_a_nas_deletes_its_history() {
        let backend = FakeBackend::answering(vec![Ok(hot_snapshot(55.0, T0))]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("state");
        assert!(!h.core.events(None, T0, T0).expect("events").is_empty());

        h.core
            .clone()
            .remove_nas("nas-1".into())
            .await
            .expect("remove");
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add again");
        assert!(
            h.core
                .history("nas-1".into(), Metric::DiskTemp, HistoryRange::Year)
                .expect("history")
                .is_empty()
        );
        assert!(h.core.events(None, 0, i64::MAX).expect("events").is_empty());
    }

    #[test]
    fn reads_of_an_unknown_nas_are_empty() {
        let h = harness(FakeBackend::answering(Vec::new()));
        assert!(
            h.core
                .history("nope".into(), Metric::Cpu, HistoryRange::Week)
                .expect("history")
                .is_empty()
        );
        assert!(
            h.core
                .events(Some("nope".into()), 0, i64::MAX)
                .expect("events")
                .is_empty()
        );
        assert_eq!(
            h.core
                .forecast("nope".into(), "vol".into())
                .expect("forecast"),
            None
        );
        assert!(
            h.core
                .backup_runs("nope".into(), 0, i64::MAX)
                .expect("runs")
                .is_empty()
        );
    }

    #[test]
    fn a_history_file_that_cannot_be_opened_is_an_error() {
        let dir = std::env::temp_dir().join(format!("nas-stats-ffi-{}", std::process::id()));
        let path = dir.join("missing").join("history.sqlite");
        assert!(matches!(
            NasCore::with_history(path.to_string_lossy().into_owned()),
            Err(FfiError::History { .. })
        ));
    }

    /// 14 November 2026, 14:05 UTC.
    const DEMO_NOW: i64 = 1_794_665_100;

    #[test]
    fn the_demo_core_has_history_events_runs_and_forecast() {
        let core = NasCore::demo(DEMO_NOW);
        let ids: Vec<String> = core.states().into_iter().map(|s| s.nas_id).collect();
        assert_eq!(ids, ["demo-buero", "demo-homelab", "demo-keller"]);

        let forecast = core
            .forecast("demo-keller".into(), "demo-keller-datavol1".into())
            .expect("forecast")
            .expect("trend");
        assert_eq!(forecast.days_to_full.round(), 143.0);
        assert!(forecast.growth_per_day > 0.0);
        assert!(forecast.full_at > DEMO_NOW);

        let used = core
            .history(
                "demo-keller".into(),
                Metric::VolumeUsed,
                HistoryRange::Month,
            )
            .expect("history");
        let datavol1 = used
            .iter()
            .find(|s| s.key.as_deref() == Some("demo-keller-datavol1"))
            .expect("DataVol1");
        assert!(datavol1.points.len() >= 30);
        assert!(datavol1.points.iter().all(|p| p.count >= 1));

        let events = core
            .events(None, DEMO_NOW - 2 * DAY, DEMO_NOW)
            .expect("events");
        let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
        use crate::records::EventKind as K;
        assert_eq!(
            kinds,
            [
                K::DiskTemperature,
                K::VolumeFill,
                K::UpsOnBattery,
                K::Unreachable
            ]
        );
        assert_eq!(events[0].nas_id, "demo-keller");
        assert_eq!(events[0].ended_at, None);
        assert_eq!(events[3].nas_id, "demo-homelab");
        assert!(events[3].ended_at.is_some());

        let runs = core
            .backup_runs("demo-keller".into(), DEMO_NOW - 7 * DAY, DEMO_NOW)
            .expect("runs");
        assert!(!runs.is_empty());
        assert!(runs.iter().all(|r| r.task_id.starts_with("demo-keller-")));
        assert!(
            core.backup_runs("demo-buero".into(), DEMO_NOW - 7 * DAY, DEMO_NOW)
                .expect("runs")
                .iter()
                .all(|r| r.task_id.starts_with("demo-buero-"))
        );
    }

    #[test]
    fn the_demo_core_answers_with_marked_demo_snapshots() {
        let core = Arc::new(NasCore::demo(DEMO_NOW));
        let (listener, states) = listener();
        core.start_planner(listener);
        let state = states.recv_timeout(WAIT).expect("callback");
        let snapshot = state.snapshot.expect("demo snapshot");
        assert!(snapshot.demo);
        assert_eq!(snapshot.nas.id, state.nas_id);
        assert!(snapshot.observed_at >= DEMO_NOW);
        core.stop_planner();
    }

    #[tokio::test]
    async fn a_query_of_a_retired_entry_sends_nothing() {
        let backend = FakeBackend::answering(vec![Ok(model_snapshot(10.0))]);
        let entry = Arc::new(Entry {
            backend: backend.clone(),
            provider: model::ProviderKind::Qnap,
            poll: tokio::sync::Mutex::new(()),
            epoch: AtomicU64::new(0),
        });
        let polled = Polled::current(Arc::clone(&entry));
        entry.retire();
        assert!(matches!(
            Provider::snapshot(polled.as_ref()).await,
            Err(ProviderError::Unreachable(_))
        ));
        assert_eq!(backend.queries(), 0);
        let fresh = Polled::current(entry);
        assert!(Provider::snapshot(fresh.as_ref()).await.is_ok());
        assert_eq!(backend.queries(), 1);
    }

    #[tokio::test]
    async fn a_refused_login_is_no_unreachable_nas() {
        let backend = FakeBackend::answering(vec![
            Ok(model_snapshot(10.0)),
            Err(ProviderError::LoginFailed),
        ]);
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        h.core.clone().refresh("nas-1".into()).await.expect("first");
        h.set_time(T0 + 4 * MIN_INTERVAL_SECS);
        let state = h
            .core
            .clone()
            .refresh("nas-1".into())
            .await
            .expect("refused");
        assert!(state.blocked);
        assert!(h.core.events(None, 0, i64::MAX).expect("events").is_empty());
    }
}
