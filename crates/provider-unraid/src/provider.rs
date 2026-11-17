//! [`nas_model::Provider`] for an Unraid server: one snapshot from all
//! queries.
//!
//! The queries go out one after the other, never in parallel: first the
//! required one (server, array, disks), then metrics, shares,
//! notifications, UPS and containers. An error of the required query fails
//! the snapshot. For an optional query an unreadable answer or GraphQL
//! error (`Parse`), an HTTP error status or missing rights leave only that
//! part empty (and are logged); every other error (unreachable, timeout,
//! refused key, changed certificate) fails the snapshot, so the app keeps
//! its last state. Network throughput, fans and temperatures of system and
//! CPU stay empty: the API of Unraid 7.2.0 does not report them.

use std::time::{SystemTime, UNIX_EPOCH};

use nas_model::{BackupRun, BoxFuture, Nas, Provider, ProviderError, ProviderKind, Snapshot};
use serde_json::Value;

use crate::client::UnraidClient;
use crate::map;
use crate::queries;
use crate::transport::Transport;
use crate::wire::{
    DockerData, MainData, MetricsData, NotificationsData, ParityHistoryData, SharesData, UpsData,
    decode,
};

/// One Unraid server as a [`Provider`]. Keeps the local id and the base URL
/// the app assigned, and the client with the API key.
#[derive(Debug)]
pub struct UnraidProvider<T: Transport> {
    nas_id: String,
    base_url: String,
    client: UnraidClient<T>,
}

impl<T: Transport> UnraidProvider<T> {
    /// Provider for the server with the local id `nas_id` at `base_url`.
    /// Sends nothing yet.
    pub fn new(
        nas_id: impl Into<String>,
        base_url: impl Into<String>,
        client: UnraidClient<T>,
    ) -> Self {
        Self {
            nas_id: nas_id.into(),
            base_url: base_url.into(),
            client,
        }
    }

    /// The client, for user actions such as changing the API key.
    pub fn client(&self) -> &UnraidClient<T> {
        &self.client
    }

    /// Answer of an optional query; see the module doc for which errors
    /// leave it empty.
    async fn optional(&self, name: &str, query: &str) -> Result<Option<Value>, ProviderError> {
        match self.client.query(query).await {
            Ok(data) => Ok(Some(data)),
            Err(e @ (ProviderError::Parse(_) | ProviderError::Http(_))) => {
                log::warn!("Unraid {name}: {e}, leaving it empty");
                Ok(None)
            }
            Err(ProviderError::PermissionDenied) => {
                log::warn!("Unraid {name}: no rights for the API key, leaving it empty");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    async fn read(&self) -> Result<Snapshot, ProviderError> {
        let main: MainData = decode(self.client.query(queries::MAIN).await?);
        let metrics: MetricsData = self
            .optional("metrics", queries::METRICS)
            .await?
            .map(decode)
            .unwrap_or_default();
        let shares: SharesData = self
            .optional("shares", queries::SHARES)
            .await?
            .map(decode)
            .unwrap_or_default();
        let notifications: NotificationsData = self
            .optional("notifications", queries::NOTIFICATIONS)
            .await?
            .map(decode)
            .unwrap_or_default();
        let ups: UpsData = self
            .optional("UPS", queries::UPS)
            .await?
            .map(decode)
            .unwrap_or_default();
        let docker: DockerData = self
            .optional("docker", queries::DOCKER)
            .await?
            .map(decode)
            .unwrap_or_default();

        let now = now();
        let info = main.info.as_ref();
        let mut volumes = Vec::new();
        let mut disks = Vec::new();
        let mut backups = Vec::new();
        if let Some(array) = &main.array {
            volumes.push(map::array_volume(array));
            disks = map::disks(array, &self.nas_id);
            if map::has_parity(array) {
                backups.push(map::parity_task(array.parity_check_status.as_ref()));
            }
        }
        volumes.extend(shares.shares.iter().filter_map(map::share_volume));
        Ok(Snapshot {
            nas: Nas {
                id: self.nas_id.clone(),
                provider: ProviderKind::Unraid,
                name: main.vars.and_then(|v| v.name),
                base_url: self.base_url.clone(),
                model: map::model(info),
                firmware: map::firmware(info),
                hostname: map::hostname(info),
            },
            observed_at: now,
            system: map::system(info, metrics.metrics.as_ref(), now),
            ups: map::ups(&ups.ups_devices),
            volumes,
            disks,
            fans: Vec::new(),
            backups,
            containers: docker
                .docker
                .map(|d| d.containers.iter().filter_map(map::container).collect())
                .unwrap_or_default(),
            alerts: notifications
                .notifications
                .map(|n| n.list.iter().filter_map(map::alert).collect())
                .unwrap_or_default(),
            interfaces: Vec::new(),
        })
    }

    async fn parity_runs(&self, since: i64) -> Result<Vec<BackupRun>, ProviderError> {
        let data = match self.client.query(queries::PARITY_HISTORY).await {
            Ok(data) => data,
            // Unraid answers a GraphQL error while there is no parity log.
            Err(ProviderError::Parse(detail)) => {
                log::info!("Unraid parity history: {detail}, no runs");
                return Ok(Vec::new());
            }
            Err(e) => return Err(e),
        };
        let history: ParityHistoryData = decode(data);
        let mut runs: Vec<BackupRun> = history
            .parity_history
            .iter()
            .filter_map(map::parity_run)
            .filter(|run| run.started_at >= since)
            .collect();
        runs.sort_by_key(|run| run.started_at);
        Ok(runs)
    }
}

impl<T: Transport> Provider for UnraidProvider<T> {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Unraid
    }

    fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
        Box::pin(self.read())
    }

    /// Past parity checks that began at or after `since`, oldest first.
    /// Without a parity log the list is empty.
    fn backup_runs(&self, since: i64) -> BoxFuture<'_, Result<Vec<BackupRun>, ProviderError>> {
        Box::pin(self.parity_runs(since))
    }
}

/// Now in Unix seconds UTC; 0 for a clock before 1970.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
