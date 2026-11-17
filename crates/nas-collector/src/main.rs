//! `nas-collector`: the collector of NAS Stats for a Docker container on
//! the NAS (#46). It queries every NAS of its configuration with the
//! planner of the core, at most every 5 minutes, keeps the history in
//! SQLite and hands it to the apps over HTTPS in the home network.
//!
//! Read only, like the apps: it sends nothing to a NAS but the queries, and
//! after a rejected login it stops until the access in the configuration
//! changes, also across restarts.
//!
//! Commands:
//! - `run` (default): query and serve until SIGTERM or SIGINT.
//! - `fingerprint <https://…>`: prints the fingerprint of the certificate of
//!   a NAS, for the configuration.
//! - `pairing`: prints fingerprint and key of the collector, for the app.
//!
//! Files: the configuration at `$NAS_COLLECTOR_CONFIG` (default
//! `/config/collector.toml`), data in `$NAS_COLLECTOR_DATA` (default
//! `/data`): `history.sqlite`, `poll-state.json`, `tls/`, `token`.

mod config;
mod pairing;
mod server;
mod state;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nas_http::NasClient;
use nas_model::{Provider, ProviderKind, Thresholds};
use provider_qnap::{QnapClient, QnapProvider};
use provider_unraid::{UnraidClient, UnraidProvider};
use stats_core::{Clock, HistoryStore, Planner, PollSink, PollState, archive_poll, system_clock};
use tokio::sync::Notify;

use crate::config::NasConfig;
use crate::state::StateFile;

const HELP: &str = "\
nas-collector: Sammel-Container von NAS Stats

Befehle:
  run                    NAS abfragen und den Verlauf bereitstellen (Standard)
  fingerprint <https://…> Fingerabdruck des Zertifikats eines NAS ausgeben
  pairing                Fingerabdruck und Schlüssel für die App ausgeben

Umgebung:
  NAS_COLLECTOR_CONFIG   Konfiguration (Standard /config/collector.toml)
  NAS_COLLECTOR_DATA     Datenordner (Standard /data)";

/// Default data directory.
const DEFAULT_DATA: &str = "/data";
/// Name of the archive in the data directory.
const HISTORY_FILE: &str = "history.sqlite";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["run"] => run(),
        ["fingerprint", url] => fingerprint(url),
        ["pairing"] => print_pairing(),
        ["help" | "--help" | "-h"] => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{HELP}");
            ExitCode::from(2)
        }
    }
}

fn config_path() -> PathBuf {
    std::env::var_os("NAS_COLLECTOR_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(config::DEFAULT_PATH))
}

fn data_dir() -> PathBuf {
    std::env::var_os("NAS_COLLECTOR_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA))
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nas-collector")
        .enable_all()
        .build()
}

/// A line of the log with the time in UTC.
fn log(text: &str) {
    let now = (system_clock())();
    println!("{} {text}", utc(now));
}

/// `YYYY-MM-DD HH:MM:SSZ` of Unix seconds.
fn utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn fingerprint(url: &str) -> ExitCode {
    let result = runtime().map_err(|e| e.to_string()).and_then(|rt| {
        rt.block_on(nas_http::first_contact(url))
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(fingerprint) => {
            println!("{fingerprint}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Fingerabdruck nicht lesbar: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_pairing() -> ExitCode {
    let dir = data_dir();
    match pairing::ensure(&dir) {
        Ok(pairing) => {
            println!("Fingerabdruck: {}", pairing.fingerprint());
            println!("Schlüssel:     {}", pairing.token);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Kopplung in {} nicht lesbar: {e}", dir.display());
            ExitCode::FAILURE
        }
    }
}

/// Gets every finished query: archive, state file, log line.
struct Sink {
    store: Mutex<HistoryStore>,
    state: StateFile,
    clock: Clock,
}

impl PollSink for Sink {
    fn polled(&self, state: &PollState) {
        let now = (self.clock)();
        {
            let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(e) = archive_poll(&mut store, state, &Thresholds::default(), now) {
                log(&format!("{}: Verlauf nicht geschrieben: {e}", state.nas_id));
            }
        }
        if let Err(e) = self.state.record(state) {
            log(&format!(
                "{}: Abfragestand nicht gespeichert: {e}",
                state.nas_id
            ));
        }
        match &state.error {
            None => log(&format!("{}: ok", state.nas_id)),
            Some(error) if state.blocked => log(&format!(
                "{}: {error}; angehalten, bis sich der Zugang in der Konfiguration ändert",
                state.nas_id
            )),
            Some(error) => log(&format!("{}: {error}", state.nas_id)),
        }
    }
}

/// A connected NAS: its provider, and how its session ends.
enum Connected {
    Qnap(Arc<QnapProvider<NasClient>>),
    /// No session: the API key needs no logout.
    Unraid(Arc<UnraidProvider<NasClient>>),
}

impl Connected {
    fn provider(&self) -> Arc<dyn Provider> {
        match self {
            Connected::Qnap(p) => Arc::clone(p) as Arc<dyn Provider>,
            Connected::Unraid(p) => Arc::clone(p) as Arc<dyn Provider>,
        }
    }

    /// Ends the session on the NAS, if there is one; the answer is ignored.
    async fn logout(&self) {
        if let Connected::Qnap(p) = self {
            let _ = p.client().logout().await;
        }
    }
}

/// Provider for one NAS, as the app builds it. Needs a runtime context.
fn provider(nas: &NasConfig) -> Result<Connected, String> {
    let http = NasClient::new(&nas.base_url, nas.fingerprint)
        .map_err(|e| format!("nas {:?}: base_url: {e}", nas.id))?;
    Ok(match nas.provider {
        // The configuration carries the API key in `password`.
        ProviderKind::Unraid => Connected::Unraid(Arc::new(UnraidProvider::new(
            nas.id.clone(),
            nas.base_url.clone(),
            UnraidClient::new(http, nas.password.clone()),
        ))),
        _ => Connected::Qnap(Arc::new(QnapProvider::new(
            nas.id.clone(),
            nas.base_url.clone(),
            QnapClient::new(http, nas.user.clone(), nas.password.clone()),
        ))),
    })
}

fn run() -> ExitCode {
    let path = config_path();
    let config = match config::load(&path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Konfiguration: {e}");
            return ExitCode::FAILURE;
        }
    };
    let dir = data_dir();
    match runtime() {
        Ok(rt) => match rt.block_on(serve(config, &dir)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("tokio: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(config: config::Config, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let pairing =
        pairing::ensure(dir).map_err(|e| format!("Kopplung in {}: {e}", dir.display()))?;
    let history = dir.join(HISTORY_FILE);
    let open = || HistoryStore::open(&history).map_err(|e| format!("{}: {e}", history.display()));
    let writer = open()?;
    let reader = open()?;

    let access: BTreeMap<String, String> = config
        .nas
        .iter()
        .map(|nas| (nas.id.clone(), nas.access()))
        .collect();
    let (state, warning) = StateFile::open(dir, access);
    if let Some(warning) = warning {
        log(&warning);
    }

    let mut providers = Vec::new();
    for nas in &config.nas {
        providers.push((nas, provider(nas)?));
    }

    let clock = system_clock();
    let sink = Arc::new(Sink {
        store: Mutex::new(writer),
        state,
        clock: Arc::clone(&clock),
    });
    let planner = Planner::new(
        i64::from(config.interval_minutes) * 60,
        Arc::clone(&clock),
        Arc::clone(&sink) as Arc<dyn PollSink>,
    );
    for (nas, provider) in &providers {
        let saved = sink.state.saved(&nas.id);
        let (last_attempt_at, blocked) = state::start_values(saved.as_ref(), &nas.access());
        if blocked {
            log(&format!(
                "{}: angehalten nach abgelehnter Anmeldung; Zugang in der Konfiguration ändern, um fortzusetzen",
                nas.id
            ));
        }
        planner.add(&nas.id, provider.provider(), last_attempt_at, blocked);
    }

    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .map_err(|e| format!("listen {}: {e}", config.listen))?;
    let tls = server::acceptor(&pairing).map_err(|e| format!("TLS: {e}"))?;
    let api = Arc::new(server::Api::new(
        config.nas.clone(),
        Arc::clone(&planner),
        reader,
        &pairing.token,
        Arc::clone(&clock),
    ));
    let stop = Arc::new(Notify::new());
    let server = tokio::spawn(server::serve(listener, tls, api, Arc::clone(&stop)));
    let planning = tokio::spawn(Arc::clone(&planner).run());
    log(&format!(
        "gestartet: {} NAS, alle {} Minuten, Schnittstelle auf {}",
        config.nas.len(),
        config.interval_minutes,
        config.listen
    ));

    wait_for_signal().await?;
    log("beendet");
    planner.stop();
    stop.notify_one();
    let _ = server.await;
    let _ = planning.await;
    // Queries in flight still report, so a refused login is kept as a
    // block before the process ends.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while planner.states().iter().any(|state| state.running)
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // End the sessions on the NAS, without waiting long for them.
    for (_, provider) in &providers {
        let _ = tokio::time::timeout(Duration::from_secs(5), provider.logout()).await;
    }
    Ok(())
}

/// Waits for SIGTERM (`docker stop`) or SIGINT.
async fn wait_for_signal() -> Result<(), String> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).map_err(|e| format!("SIGTERM: {e}"))?;
    let mut int = signal(SignalKind::interrupt()).map_err(|e| format!("SIGINT: {e}"))?;
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nas_model::{BoxFuture, Provider, ProviderError, ProviderKind, Snapshot};
    use stats_core::Manual;

    const T0: i64 = 1_800_000_000;

    /// A NAS that refuses every login and counts the attempts.
    #[derive(Default)]
    struct Refusing {
        logins: AtomicUsize,
    }

    impl Provider for Refusing {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Qnap
        }
        fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
            self.logins.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(Err(ProviderError::LoginFailed)))
        }
    }

    /// One start of the collector against `dir`: sink and planner as `serve`
    /// builds them, with `nas` registered.
    fn start(dir: &Path, nas: &NasConfig, provider: Arc<Refusing>, now: i64) -> Arc<Planner> {
        let access = BTreeMap::from([(nas.id.clone(), nas.access())]);
        let (state, _) = StateFile::open(dir, access);
        let clock: Clock = Arc::new(move || now);
        let sink = Arc::new(Sink {
            store: Mutex::new(HistoryStore::in_memory().expect("store")),
            state,
            clock: Arc::clone(&clock),
        });
        let planner = Planner::new(300, clock, Arc::clone(&sink) as Arc<dyn PollSink>);
        let saved = sink.state.saved(&nas.id);
        let (last, blocked) = state::start_values(saved.as_ref(), &nas.access());
        planner.add(&nas.id, provider, last, blocked);
        planner
    }

    #[tokio::test]
    async fn rejected_login_is_not_repeated_after_a_restart() {
        let dir =
            std::env::temp_dir().join(format!("nas-collector-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let text = include_str!("../../../docker/collector/collector.example.toml");
        let mut nas = config::parse(text).expect("config").nas.remove(0);
        let provider = Arc::new(Refusing::default());

        let planner = start(&dir, &nas, Arc::clone(&provider), T0);
        let state = planner.refresh_now(&nas.id).await.expect("queried");
        assert!(state.blocked);
        assert_eq!(provider.logins.load(Ordering::SeqCst), 1);

        // Two restarts with the same access: no query, not even a due one.
        for later in [600, 3_600] {
            let planner = start(&dir, &nas, Arc::clone(&provider), T0 + later);
            assert!(planner.state(&nas.id).expect("state").blocked);
            assert_eq!(planner.refresh_now(&nas.id).await, Err(Manual::Blocked));
        }
        assert_eq!(provider.logins.load(Ordering::SeqCst), 1);

        // A new password lifts the block.
        nas.password = "neues-passwort".into();
        let planner = start(&dir, &nas, Arc::clone(&provider), T0 + 7_200);
        assert!(!planner.state(&nas.id).expect("state").blocked);
        planner.refresh_now(&nas.id).await.expect("queried");
        assert_eq!(provider.logins.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn utc_formats_known_times() {
        assert_eq!(utc(0), "1970-01-01 00:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(utc(1_800_000_000), "2027-01-15 08:00:00Z");
    }
}
