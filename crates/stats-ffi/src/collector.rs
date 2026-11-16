//! The collector on the NAS (#46): a Docker container that queries the NAS
//! around the clock and hands its history out over HTTPS in the home
//! network. The app pins the certificate of the collector like that of a
//! NAS and sends the key of the collector as a bearer token.
//!
//! [`collector_nas`] lists the NAS of a collector,
//! [`NasCore::import_from_collector`] takes over the history of one of them
//! under the id of a NAS of the app. Pairing and the matching of NAS are
//! up to the app (#124).

use std::sync::Arc;
use std::time::Duration;

use nas_http::{CONNECT_TIMEOUT, Fingerprint, NasClient};
use nas_model::ProviderError;
use stats_core::Export;

use crate::core::NasCore;
use crate::error::FfiError;
use crate::records::{CollectorImport, CollectorNas};
use crate::{on_core, runtime};

/// Time limit of a request to the collector: a first export of a year of
/// history is large.
const COLLECTOR_TIMEOUT: Duration = Duration::from_secs(120);

/// Margin of a follow-up import, seconds: values that reached the archive
/// of the collector shortly before its last export come again.
const SINCE_MARGIN_SECS: i64 = 3_600;

/// Client for the collector at `base_url` with its pinned certificate.
fn client(base_url: &str, fingerprint: &str) -> Result<NasClient, FfiError> {
    let pin: Fingerprint =
        fingerprint
            .parse()
            .map_err(|e: ProviderError| FfiError::InvalidInput {
                detail: e.to_string(),
            })?;
    // The HTTP client belongs to the core runtime.
    let _runtime = runtime().enter();
    NasClient::with_timeouts(base_url, pin, COLLECTOR_TIMEOUT, CONNECT_TIMEOUT).map_err(|e| {
        FfiError::InvalidInput {
            detail: e.to_string(),
        }
    })
}

/// An answer of the collector that does not parse.
fn parse_error(e: serde_json::Error) -> FfiError {
    FfiError::Parse {
        detail: e.to_string(),
    }
}

/// The NAS of the collector at `base_url` (`https://<nas>:8743`), whose
/// certificate has `fingerprint` and whose key is `token` (both from
/// `nas-collector pairing`). A rejected key is `LoginFailed`, an answer
/// that does not parse `Parse`.
#[uniffi::export]
pub async fn collector_nas(
    base_url: String,
    fingerprint: String,
    token: String,
) -> Result<Vec<CollectorNas>, FfiError> {
    let client = client(&base_url, &fingerprint)?;
    on_core(async move {
        let body = client.get_text_bearer("v1/nas", &[], &token).await?;
        let list: Vec<stats_core::CollectorNas> =
            serde_json::from_str(&body).map_err(parse_error)?;
        Ok(list.into_iter().map(Into::into).collect())
    })
    .await
}

#[uniffi::export]
impl NasCore {
    /// Takes over the history of the NAS `collector_nas_id` of the collector
    /// at `base_url` (see [`collector_nas`]) into the archive, under the
    /// registered NAS `nas_id` of the app: samples, hourly and daily values
    /// and backup runs, no events. `since` is the `until` of the last
    /// import (`None`: everything); the collector sends from an hour before
    /// it on, values already stored count once. Save the returned `until`
    /// for the next import.
    ///
    /// A NAS of the app that is not registered is `UnknownNas`, a rejected
    /// key `LoginFailed`, an unknown NAS at the collector `Http` 404, an
    /// answer that does not parse `Parse`.
    pub async fn import_from_collector(
        self: Arc<Self>,
        base_url: String,
        fingerprint: String,
        token: String,
        collector_nas_id: String,
        nas_id: String,
        since: Option<i64>,
    ) -> Result<CollectorImport, FfiError> {
        self.check_registered(&nas_id)?;
        let client = client(&base_url, &fingerprint)?;
        on_core(async move {
            let since = since.map(|at| at.saturating_sub(SINCE_MARGIN_SECS).max(0).to_string());
            let mut query = vec![("nas", collector_nas_id.as_str())];
            if let Some(since) = &since {
                query.push(("since", since.as_str()));
            }
            let body = client.get_text_bearer("v1/export", &query, &token).await?;
            let export: Export = serde_json::from_str(&body).map_err(parse_error)?;
            let count = self.import_export(&nas_id, &export)?;
            Ok(CollectorImport {
                until: export.until,
                samples: count.samples,
                hourly: count.hourly,
                daily: count.daily,
                backup_runs: count.backup_runs,
            })
        })
        .await
    }
}
