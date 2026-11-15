//! Swift face of the NAS Stats core, via UniFFI.
//!
//! Keeps its own records (`records.rs`) so `nas-model` stays free of FFI
//! attributes. [`NasCore`] holds the registered NAS, queries them and keeps
//! their last state and error. Free calls: [`core_version`],
//! [`fetch_fingerprint`] and [`demo_snapshot`].
//!
//! Every network call runs on one tokio runtime of the core ([`runtime`]);
//! the apps only await the result. History, forecast, events, the planner
//! and a callback to Swift follow with #60.

mod core;
mod demo;
mod error;
mod records;

pub use crate::core::{MIN_INTERVAL_SECS, NasCore};
pub use crate::error::FfiError;
pub use crate::records::{
    Alert, AlertLevel, BackupKind, BackupResult, BackupTask, Container, Disk, Fan, Health, Nas,
    NasConfig, NasProblem, NasState, NetInterface, ProblemKind, ProviderKind, ScrubState,
    ScrubStatus, Snapshot, SystemStatus, Ups, UpsState, Volume, VolumeKind,
};

uniffi::setup_scaffolding!();

/// One tokio runtime for every network call of the core; the apps never
/// see it.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("nasstats-core")
            .enable_all()
            .build()
            .unwrap_or_else(|err| panic!("cannot start the tokio runtime: {err}"))
    })
}

/// Runs a future on the core runtime; the Swift side only awaits the
/// handle. A task that panics becomes an error instead of taking the app
/// down.
pub(crate) async fn on_core<T: Send + 'static>(
    fut: impl std::future::Future<Output = Result<T, FfiError>> + Send + 'static,
) -> Result<T, FfiError> {
    runtime().spawn(fut).await.unwrap_or_else(|e| {
        Err(FfiError::Internal {
            detail: format!("Core-Aufgabe abgebrochen: {e}"),
        })
    })
}

/// Version of the Rust core, as the apps show it ("Core 0.1.0").
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Reads the certificate of the NAS at `base_url` (`https://…`) without
/// sending a request and returns its SHA-256 fingerprint as `AB:CD:…`, for
/// the user to confirm.
#[uniffi::export]
pub async fn fetch_fingerprint(base_url: String) -> Result<String, FfiError> {
    on_core(async move {
        match nas_http::first_contact(&base_url).await {
            Ok(fingerprint) => Ok(fingerprint.to_string()),
            // Before any connection only the address can be wrong, as for
            // `NasCore::add_nas`.
            Err(nas_model::ProviderError::Parse(detail)) => Err(FfiError::InvalidInput { detail }),
            Err(e) => Err(e.into()),
        }
    })
    .await
}

/// A fully filled snapshot of an invented NAS, with `demo` set.
#[uniffi::export]
pub fn demo_snapshot() -> Snapshot {
    Snapshot::demo(demo::snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_is_the_workspace_version() {
        assert_eq!(core_version(), "0.1.0");
    }

    #[test]
    fn demo_snapshot_is_marked_and_full() {
        let s = demo_snapshot();
        assert!(s.demo);
        assert_eq!(s.nas.id, "demo-keller");
        assert_eq!(s.nas.model.as_deref(), Some("TS-464"));
        assert_eq!(s.nas.firmware.as_deref(), Some("QTS 5.2.4"));
        let data = s
            .volumes
            .iter()
            .find(|v| v.name == "DataVol1")
            .expect("DataVol1");
        let used = data.used_percent.expect("used percent");
        assert!((used - 87.1).abs() < 1e-9);
        let disk3 = s.disks.iter().find(|d| d.id == "disk-3").expect("disk 3");
        assert_eq!(disk3.temperature_c, Some(52.0));
        assert_eq!(disk3.serial_hash.as_ref().map(String::len), Some(64));
        assert_eq!(s.worst_health, Some(Health::Warning));

        // Every list holds an entry, every optional field is set.
        assert!(s.ups.is_some());
        for list_len in [
            s.volumes.len(),
            s.disks.len(),
            s.fans.len(),
            s.backups.len(),
            s.containers.len(),
            s.alerts.len(),
            s.interfaces.len(),
        ] {
            assert!(list_len >= 1);
        }
        let model = demo::snapshot();
        let json = format!("{model:?}");
        assert!(!json.contains("None"), "demo leaves a field empty: {json}");
    }

    #[tokio::test]
    async fn fetch_fingerprint_refuses_http() {
        assert!(matches!(
            fetch_fingerprint("http://nas.example".into()).await,
            Err(FfiError::InvalidInput { .. })
        ));
    }
}
