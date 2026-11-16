//! Planner with a simulated clock (paused tokio time) and a fake provider.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nas_model::{BoxFuture, Nas, Provider, ProviderError, ProviderKind, Snapshot, SystemStatus};
use stats_core::{Clock, Manual, Planner, PollSink, PollState};
use tokio::time::Instant;

const T0: i64 = 1_800_000_000;

fn snapshot() -> Snapshot {
    Snapshot {
        nas: Nas {
            id: "nas-1".into(),
            provider: ProviderKind::Qnap,
            name: None,
            base_url: "https://nas.example".into(),
            model: Some("TS-000".into()),
            firmware: None,
            hostname: None,
        },
        observed_at: T0,
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

/// Answers from a queue (empty: a snapshot), counts the queries.
#[derive(Default)]
struct FakeProvider {
    answers: Mutex<VecDeque<Result<Snapshot, ProviderError>>>,
    calls: AtomicUsize,
    delay: Duration,
}

impl FakeProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn answering(answers: impl IntoIterator<Item = Result<Snapshot, ProviderError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            ..Self::default()
        })
    }

    fn slow(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            delay,
            ..Self::default()
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Provider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Qnap
    }

    fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let answer = self
                .answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            answer.unwrap_or_else(|| Ok(snapshot()))
        })
    }
}

#[derive(Default)]
struct Recorder {
    states: Mutex<Vec<PollState>>,
}

impl Recorder {
    fn count(&self) -> usize {
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl PollSink for Recorder {
    fn polled(&self, state: &PollState) {
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(state.clone());
    }
}

/// `T0` plus the seconds of tokio time since the start.
fn clock(start: Instant) -> Clock {
    Arc::new(move || T0 + i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX))
}

struct Harness {
    start: Instant,
    planner: Arc<Planner>,
    sink: Arc<Recorder>,
}

impl Harness {
    fn new(interval_secs: i64) -> Self {
        let start = Instant::now();
        let sink = Arc::new(Recorder::default());
        let planner = Planner::new(interval_secs, clock(start), sink.clone());
        Self {
            start,
            planner,
            sink,
        }
    }

    fn run(&self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(Arc::clone(&self.planner).run())
    }

    /// Lets the simulated time run up to `secs` after the start.
    async fn at(&self, secs: u64) {
        tokio::time::sleep_until(self.start + Duration::from_secs(secs)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn one_hour_gives_12_queries_at_5_minutes_and_6_at_10() {
    for (interval, expected) in [(300, 12), (600, 6), (60, 12)] {
        let harness = Harness::new(interval);
        let provider = FakeProvider::new();
        harness.planner.add("nas-1", provider.clone(), None, false);
        let task = harness.run();
        harness.at(3599).await;
        assert_eq!(provider.calls(), expected, "interval {interval}");
        assert_eq!(harness.sink.count(), expected, "interval {interval}");
        harness.planner.stop();
        task.await.expect("planner ends");
    }
}

#[tokio::test(start_paused = true)]
async fn rejected_login_stops_only_that_nas() {
    let harness = Harness::new(300);
    let refused = FakeProvider::answering([Err(ProviderError::LoginFailed)]);
    let fine = FakeProvider::new();
    harness.planner.add("nas-a", refused.clone(), None, false);
    harness.planner.add("nas-b", fine.clone(), None, false);
    let task = harness.run();
    harness.at(3599).await;

    assert_eq!(refused.calls(), 1);
    assert_eq!(fine.calls(), 12);
    assert_eq!(harness.sink.count(), 13);
    let state = harness.planner.state("nas-a").expect("registered");
    assert!(state.blocked);
    assert_eq!(state.error, Some(ProviderError::LoginFailed));
    assert_eq!(
        harness.planner.refresh_now("nas-a").await,
        Err(Manual::Blocked)
    );

    // Unblocked by the user: queried at once, then by the interval.
    assert!(harness.planner.unblock("nas-a"));
    harness.at(3600).await;
    assert_eq!(refused.calls(), 2);
    assert!(!harness.planner.state("nas-a").expect("registered").blocked);

    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn network_error_waits_for_the_regular_time() {
    let harness = Harness::new(300);
    let provider =
        FakeProvider::answering([Ok(snapshot()), Err(ProviderError::Unreachable("x".into()))]);
    harness.planner.add("nas-1", provider.clone(), None, false);
    let task = harness.run();

    harness.at(299).await;
    assert_eq!(provider.calls(), 1);
    harness.at(301).await;
    assert_eq!(provider.calls(), 2);
    let state = harness.planner.state("nas-1").expect("registered");
    assert_eq!(state.snapshot, Some(snapshot()));
    assert_eq!(state.error_at, Some(T0 + 300));
    assert_eq!(
        state.error.map(|e| e.to_string()).as_deref(),
        Some("NAS nicht erreichbar: x")
    );
    assert!(!state.blocked);

    harness.at(599).await;
    assert_eq!(provider.calls(), 2);
    harness.at(601).await;
    assert_eq!(provider.calls(), 3);
    assert_eq!(
        harness.planner.state("nas-1").expect("registered").error,
        None
    );

    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn manual_refresh_waits_30_seconds_and_moves_the_next_query() {
    let harness = Harness::new(300);
    let provider = FakeProvider::new();
    harness.planner.add("nas-1", provider.clone(), None, false);
    let task = harness.run();

    harness.at(10).await;
    assert_eq!(provider.calls(), 1);
    assert_eq!(
        harness.planner.refresh_now("nas-1").await,
        Err(Manual::TooSoon { retry_at: T0 + 30 })
    );
    assert_eq!(provider.calls(), 1);

    harness.at(30).await;
    let state = harness.planner.refresh_now("nas-1").await.expect("queried");
    assert_eq!(state.last_attempt_at, Some(T0 + 30));
    assert_eq!(provider.calls(), 2);
    assert_eq!(harness.sink.count(), 2);

    harness.at(329).await;
    assert_eq!(provider.calls(), 2);
    harness.at(331).await;
    assert_eq!(provider.calls(), 3);

    assert_eq!(
        harness.planner.refresh_now("unknown").await,
        Err(Manual::Unknown)
    );
    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn manual_refresh_during_a_query_is_refused() {
    let harness = Harness::new(300);
    let provider = FakeProvider::slow(Duration::from_secs(10));
    harness.planner.add("nas-1", provider.clone(), None, false);

    let planner = Arc::clone(&harness.planner);
    let first = tokio::spawn(async move { planner.refresh_now("nas-1").await });
    harness.at(1).await;
    assert_eq!(
        harness.planner.refresh_now("nas-1").await,
        Err(Manual::Running)
    );
    assert!(harness.planner.state("nas-1").expect("registered").running);

    let state = first.await.expect("task ends").expect("queried");
    assert!(!state.running);
    assert_eq!(provider.calls(), 1);
    assert_eq!(harness.sink.count(), 1);
}

#[tokio::test(start_paused = true)]
async fn removed_nas_reports_nothing() {
    let harness = Harness::new(300);
    let provider = FakeProvider::slow(Duration::from_secs(10));
    harness.planner.add("nas-1", provider.clone(), None, false);
    let task = harness.run();

    harness.at(1).await;
    assert_eq!(provider.calls(), 1);
    assert!(harness.planner.remove("nas-1"));
    harness.at(20).await;
    assert_eq!(harness.sink.count(), 0);
    assert_eq!(harness.planner.state("nas-1"), None);

    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn remembered_state_is_respected_at_start() {
    let harness = Harness::new(300);
    let recent = FakeProvider::new();
    let blocked = FakeProvider::new();
    harness
        .planner
        .add("nas-1", recent.clone(), Some(T0 - 100), false);
    harness.planner.add("nas-2", blocked.clone(), None, true);
    let task = harness.run();

    harness.at(199).await;
    assert_eq!(recent.calls(), 0);
    harness.at(201).await;
    assert_eq!(recent.calls(), 1);
    harness.at(3599).await;
    assert_eq!(blocked.calls(), 0);

    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn no_query_after_stop() {
    let harness = Harness::new(300);
    let provider = FakeProvider::new();
    harness.planner.add("nas-1", provider.clone(), None, false);
    let task = harness.run();

    harness.at(10).await;
    assert_eq!(provider.calls(), 1);
    harness.planner.stop();
    task.await.expect("planner ends");
    harness.at(1000).await;
    assert_eq!(provider.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn cancelled_manual_refresh_frees_the_nas() {
    let harness = Harness::new(300);
    let provider = FakeProvider::slow(Duration::from_secs(100));
    harness.planner.add("nas-1", provider.clone(), None, false);

    let cancelled =
        tokio::time::timeout(Duration::from_secs(5), harness.planner.refresh_now("nas-1")).await;
    assert!(cancelled.is_err());
    let state = harness.planner.state("nas-1").expect("registered");
    assert!(!state.running);
    assert_eq!(state.last_attempt_at, Some(T0));
    assert_eq!(harness.sink.count(), 0);

    // The automatic query comes at the regular time.
    let task = harness.run();
    harness.at(299).await;
    assert_eq!(provider.calls(), 1);
    harness.at(301).await;
    assert_eq!(provider.calls(), 2);
    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn replacing_during_a_query_starts_no_second_one() {
    let harness = Harness::new(300);
    let old = FakeProvider::slow(Duration::from_secs(10));
    let new = FakeProvider::new();
    harness.planner.add("nas-1", old.clone(), None, false);
    let task = harness.run();

    harness.at(1).await;
    harness.planner.add("nas-1", new.clone(), None, false);
    harness.at(5).await;
    assert_eq!(new.calls(), 0);
    harness.at(11).await;
    assert_eq!(new.calls(), 1);
    assert_eq!(harness.sink.count(), 1);

    harness.planner.stop();
    task.await.expect("planner ends");
}

/// Panics on every query.
struct PanickingProvider;

impl Provider for PanickingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Qnap
    }

    fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
        Box::pin(async { panic!("provider broke") })
    }
}

#[tokio::test(start_paused = true)]
async fn panicking_provider_does_not_stop_the_nas_for_good() {
    let harness = Harness::new(300);
    harness
        .planner
        .add("nas-1", Arc::new(PanickingProvider), None, false);
    let task = harness.run();
    harness.at(1).await;
    let state = harness.planner.state("nas-1").expect("registered");
    assert!(!state.running);
    assert_eq!(state.last_attempt_at, Some(T0));
    assert_eq!(harness.sink.count(), 0);
    harness.planner.stop();
    task.await.expect("planner ends");
}

#[tokio::test(start_paused = true)]
async fn removing_and_adding_during_a_query_starts_no_second_one() {
    let harness = Harness::new(300);
    let old = FakeProvider::slow(Duration::from_secs(10));
    let new = FakeProvider::new();
    harness.planner.add("nas-1", old.clone(), None, false);
    let task = harness.run();

    harness.at(1).await;
    assert!(harness.planner.remove("nas-1"));
    harness.planner.add("nas-1", new.clone(), None, false);
    harness.at(5).await;
    assert_eq!(new.calls(), 0);
    harness.at(11).await;
    assert_eq!(new.calls(), 1);

    harness.planner.stop();
    task.await.expect("planner ends");
}
