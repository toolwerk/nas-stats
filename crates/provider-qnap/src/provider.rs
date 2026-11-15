//! [`nas_model::Provider`] for a QNAP: one snapshot from the system values.
//!
//! For now the snapshot holds the NAS, the system values, fans, UPS and
//! alerts. Volumes, disks, backups, containers and network interfaces stay
//! empty; they follow with #52.

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

    /// Reads the system values; their error is returned unchanged.
    fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
        Box::pin(async move {
            let system = self.client.system().await?;
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
                // Follow with #52.
                volumes: Vec::new(),
                disks: Vec::new(),
                fans: system.fans,
                backups: Vec::new(),
                containers: Vec::new(),
                alerts: system.alerts,
                interfaces: Vec::new(),
            })
        })
    }
}

/// Now in Unix seconds UTC; 0 for a clock before 1970.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
