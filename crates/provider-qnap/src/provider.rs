//! [`nas_model::Provider`] for a QNAP: one snapshot from all parts.
//!
//! The parts are asked one after the other, never in parallel: system
//! values, storage, network, containers. The snapshot holds the NAS, system
//! values, fans, UPS, alerts, volumes, disks, network interfaces and
//! containers. Backups stay empty until #41, storage pools stay empty
//! because QTS does not answer them to a GET (#6).
//!
//! Failing system values fail the snapshot. For storage, network and
//! containers an HTTP error status leaves only that part empty (and is
//! logged); every other error (unreachable, timeout, rejected login, changed
//! certificate) fails the snapshot, so the app keeps its last state.

use std::time::{SystemTime, UNIX_EPOCH};

use nas_model::{BoxFuture, Nas, Provider, ProviderError, ProviderKind, Snapshot};

use crate::client::QnapClient;
use crate::transport::Transport;

/// One QNAP as a [`Provider`]. Keeps the local id and the base URL the app
/// assigned, and the client with its session.
#[derive(Debug)]
pub struct QnapProvider<T: Transport> {
    nas_id: String,
    base_url: String,
    client: QnapClient<T>,
}

impl<T: Transport> QnapProvider<T> {
    /// Provider for the NAS with the local id `nas_id` at `base_url`. Sends
    /// nothing yet.
    pub fn new(
        nas_id: impl Into<String>,
        base_url: impl Into<String>,
        client: QnapClient<T>,
    ) -> Self {
        Self {
            nas_id: nas_id.into(),
            base_url: base_url.into(),
            client,
        }
    }

    /// The client, for user actions such as changing the credentials or
    /// logging out.
    pub fn client(&self) -> &QnapClient<T> {
        &self.client
    }
}

impl<T: Transport> Provider for QnapProvider<T> {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Qnap
    }

    /// Reads system values, storage, network and containers one after the
    /// other. The error of the system values is returned unchanged; see the
    /// module doc for the other parts.
    fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
        Box::pin(async move {
            let system = self.client.system().await?;
            let storage = part("storage", self.client.storage(&self.nas_id).await)?;
            let interfaces = part("network", self.client.network().await)?;
            let containers = part("containers", self.client.containers().await)?;
            Ok(Snapshot {
                nas: Nas {
                    id: self.nas_id.clone(),
                    provider: ProviderKind::Qnap,
                    // QTS shows the server name as the name of the NAS.
                    name: system.hostname.clone(),
                    base_url: self.base_url.clone(),
                    model: system.model,
                    firmware: system.firmware,
                    hostname: system.hostname,
                },
                observed_at: now(),
                system: system.status,
                ups: system.ups,
                volumes: storage.volumes,
                disks: storage.disks,
                fans: system.fans,
                backups: Vec::new(),
                containers,
                alerts: system.alerts,
                interfaces,
            })
        })
    }
}

/// An HTTP error status of one part leaves that part empty; any other error
/// goes on to the caller.
fn part<T: Default>(name: &str, result: Result<T, ProviderError>) -> Result<T, ProviderError> {
    match result {
        Err(ProviderError::Http(status)) => {
            log::warn!("QNAP {name} answered HTTP {status}, leaving it empty");
            Ok(T::default())
        }
        other => other,
    }
}

/// Now in Unix seconds UTC; 0 for a clock before 1970.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
