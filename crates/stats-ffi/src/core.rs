//! [`NasCore`]: the NAS the app registered, their last state and the rules
//! for querying them.
//!
//! The app keeps the list of NAS and the credentials (keychain) and hands
//! every NAS to the core at each start with [`NasCore::add_nas`]; the core
//! keeps everything in memory only.
//!
//! Rules:
//!
//! - A NAS is queried at most once per [`MIN_INTERVAL_SECS`], however often
//!   the app asks; in between [`NasCore::refresh`] returns the last state
//!   without a request. A clock set back counts as the interval passed.
//! - After a rejected login, a required second factor or a changed
//!   certificate the core no longer queries the NAS until the app changes
//!   the credentials or adds the NAS again.
//! - The last snapshot stays after every error.
//!
//! So the app can show the last state after a restart, it saves the
//! snapshot of every NAS with [`NasCore::saved_state`] and hands it back
//! with [`NasCore::restore_state`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use nas_http::{Fingerprint, NasClient};
use nas_model::{self as model, BoxFuture, Provider, ProviderError};
use provider_qnap::{QnapClient, QnapProvider};

use crate::error::FfiError;
use crate::records::{NasConfig, NasProblem, NasState, ProviderKind, Snapshot};
use crate::{on_core, runtime};

/// Shortest time between two queries of the same NAS, in seconds.
pub const MIN_INTERVAL_SECS: i64 = 300;

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

/// Now in Unix seconds UTC.
pub(crate) type Clock = Box<dyn Fn() -> i64 + Send + Sync>;

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

fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
struct Status {
    snapshot: Option<Snapshot>,
    /// The snapshot as read, for [`NasCore::saved_state`].
    raw: Option<model::Snapshot>,
    problem: Option<NasProblem>,
    last_attempt_at: Option<i64>,
    blocked: bool,
}

impl Status {
    fn record(&self, nas_id: &str) -> NasState {
        NasState {
            nas_id: nas_id.to_owned(),
            snapshot: self.snapshot.clone(),
            problem: self.problem.clone(),
            last_attempt_at: self.last_attempt_at,
            blocked: self.blocked,
        }
    }

    /// Back to "never queried", the snapshot stays.
    fn reset(&mut self) {
        self.problem = None;
        self.last_attempt_at = None;
        self.blocked = false;
    }
}

struct Entry {
    backend: Arc<dyn Backend>,
    /// Held for a whole query, so a parallel call waits for its result.
    poll: tokio::sync::Mutex<()>,
    /// Never held across an `await`.
    status: Mutex<Status>,
}

/// The NAS registered by the app and their last state.
#[derive(uniffi::Object)]
pub struct NasCore {
    entries: Mutex<BTreeMap<String, Arc<Entry>>>,
    connect: Connector,
    clock: Clock,
}

impl NasCore {
    /// Core with its own factory and clock, for tests.
    pub(crate) fn with_parts(connect: Connector, clock: Clock) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
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

    /// `Some(state)` when `entry` is no longer the registered entry of
    /// `nas_id`: the state of the entry now registered, or `UnknownNas`.
    /// A removed or replaced entry is never queried and its result never
    /// reaches the app.
    fn superseded(&self, nas_id: &str, entry: &Arc<Entry>) -> Option<Result<NasState, FfiError>> {
        let entries = lock(&self.entries);
        match entries.get(nas_id) {
            Some(current) if Arc::ptr_eq(current, entry) => None,
            Some(current) => Some(Ok(lock(&current.status).record(nas_id))),
            None => Some(Err(FfiError::UnknownNas {
                id: nas_id.to_owned(),
            })),
        }
    }

    async fn refresh_entry(&self, nas_id: &str) -> Result<NasState, FfiError> {
        let entry = self.entry(nas_id)?;
        let _poll = entry.poll.lock().await;
        if let Some(answer) = self.superseded(nas_id, &entry) {
            return answer;
        }
        let now = (self.clock)();
        {
            let mut status = lock(&entry.status);
            let recent = status
                .last_attempt_at
                .is_some_and(|last| now >= last && now - last < MIN_INTERVAL_SECS);
            if status.blocked || recent {
                return Ok(status.record(nas_id));
            }
            status.last_attempt_at = Some(now);
        }
        let result = entry.backend.snapshot().await;
        // Removed or replaced while the query ran: the result is dropped.
        if let Some(answer) = self.superseded(nas_id, &entry) {
            return answer;
        }
        let mut status = lock(&entry.status);
        match result {
            Ok(snapshot) => {
                status.raw = Some(snapshot.clone());
                status.snapshot = Some(Snapshot::real(snapshot));
                status.problem = None;
            }
            Err(e) => {
                status.blocked = e.stops_polling();
                status.problem = Some(NasProblem::from(&e));
            }
        }
        Ok(status.record(nas_id))
    }
}

impl Default for NasCore {
    fn default() -> Self {
        Self::with_parts(Box::new(connect), Box::new(system_clock))
    }
}

#[uniffi::export]
impl NasCore {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self::default()
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
        let mut entries = lock(&self.entries);
        let (snapshot, raw) = entries
            .get(&config.id)
            .map(|old| {
                let old = lock(&old.status);
                (old.snapshot.clone(), old.raw.clone())
            })
            .unwrap_or_default();
        let replaced = entries.insert(
            config.id,
            Arc::new(Entry {
                backend,
                poll: tokio::sync::Mutex::new(()),
                status: Mutex::new(Status {
                    snapshot,
                    raw,
                    ..Status::default()
                }),
            }),
        );
        drop(entries);
        // The session of a replaced entry ends in the background, after a
        // query still running on it; its error is ignored.
        if let Some(old) = replaced {
            runtime().spawn(async move {
                let _poll = old.poll.lock().await;
                let _ = old.backend.logout().await;
            });
        }
        Ok(())
    }

    /// Removes a NAS and ends its session; an error of the logout is
    /// ignored.
    pub async fn remove_nas(self: Arc<Self>, nas_id: String) -> Result<(), FfiError> {
        let entry = lock(&self.entries)
            .remove(&nas_id)
            .ok_or(FfiError::UnknownNas { id: nas_id })?;
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
                if self.superseded(&nas_id, &entry).is_some() {
                    continue;
                }
                entry.backend.set_credentials(user, password).await;
                lock(&entry.status).reset();
                return Ok(());
            }
        })
        .await
    }

    /// Queries the NAS, unless it is blocked or was queried less than
    /// [`MIN_INTERVAL_SECS`] ago; then returns the last state without a
    /// request. A failed query does not throw: its error is in
    /// [`NasState::problem`], the last snapshot stays.
    pub async fn refresh(self: Arc<Self>, nas_id: String) -> Result<NasState, FfiError> {
        on_core(async move { self.refresh_entry(&nas_id).await }).await
    }

    /// State of every registered NAS, sorted by id; sends nothing.
    pub fn states(&self) -> Vec<NasState> {
        lock(&self.entries)
            .iter()
            .map(|(id, entry)| lock(&entry.status).record(id))
            .collect()
    }

    /// The last snapshot read from the NAS as JSON, for the app to save;
    /// `None` before the first successful query. Sends nothing.
    pub fn saved_state(&self, nas_id: String) -> Result<Option<String>, FfiError> {
        let entry = self.entry(&nas_id)?;
        let raw = lock(&entry.status).raw.clone();
        raw.map(|raw| {
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
        let entry = self.entry(&nas_id)?;
        let raw: model::Snapshot =
            serde_json::from_str(&json).map_err(|e| FfiError::InvalidInput {
                detail: e.to_string(),
            })?;
        if raw.nas.id != nas_id {
            return Err(FfiError::InvalidInput {
                detail: format!("saved state of {} given for {nas_id}", raw.nas.id),
            });
        }
        let mut status = lock(&entry.status);
        if status.snapshot.is_none() {
            status.snapshot = Some(Snapshot::real(raw.clone()));
            status.raw = Some(raw);
        }
        Ok(())
    }

    /// Queries a NAS once without registering it, e.g. before saving it,
    /// and logs out again (an error of the logout is ignored). Ignores the
    /// interval; an error is thrown.
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
                Box::new(move || clock.load(Ordering::SeqCst)),
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
    async fn parallel_refreshes_share_one_query() {
        let backend = Arc::new(FakeBackend {
            answers: Mutex::new(vec![Ok(model_snapshot(10.0))].into()),
            slow: true,
            ..FakeBackend::default()
        });
        let h = harness(backend.clone());
        h.core
            .add_nas(config("nas-1"), "u".into(), "p".into())
            .expect("add");
        let (a, b) = tokio::join!(
            h.core.clone().refresh("nas-1".into()),
            h.core.clone().refresh("nas-1".into())
        );
        assert_eq!(backend.queries(), 1);
        assert_eq!(cpu(&a.expect("a")), Some(10.0));
        assert_eq!(cpu(&b.expect("b")), Some(10.0));
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
}
