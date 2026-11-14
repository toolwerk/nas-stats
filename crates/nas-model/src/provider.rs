//! The interface every provider crate implements, and its errors.

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::backup::BackupRun;
use crate::nas::ProviderKind;
use crate::sample::Sample;
use crate::snapshot::Snapshot;

/// Boxed future, so that `Box<dyn Provider>` works without an async runtime.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Why a provider could not read the NAS.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    #[error("NAS nicht erreichbar")]
    Unreachable,
    #[error("Zeitüberschreitung bei der Abfrage")]
    Timeout,
    #[error("Anmeldung fehlgeschlagen")]
    LoginFailed,
    #[error("NAS verlangt einen zweiten Faktor")]
    SecondFactorRequired,
    #[error("Konto hat keine Rechte für diese Abfrage")]
    PermissionDenied,
    #[error("Zertifikat des NAS hat sich geändert (vorher {old}, jetzt {new})")]
    CertificateChanged {
        /// Fingerprint the app trusted so far.
        old: String,
        /// Fingerprint the NAS presents now.
        new: String,
    },
    #[error("Unerwartete Antwort: {0}")]
    Parse(String),
    #[error("HTTP-Status {0}")]
    Http(u16),
}

impl ProviderError {
    /// Whether polling must stop until the user acts.
    ///
    /// True for a failed login, a required second factor and a changed
    /// certificate: retrying would lock the account or the IP (DSM) or
    /// trust a foreign certificate.
    pub fn stops_polling(&self) -> bool {
        matches!(
            self,
            ProviderError::LoginFailed
                | ProviderError::SecondFactorRequired
                | ProviderError::CertificateChanged { .. }
        )
    }
}

/// Read-only access to one NAS.
///
/// Implementations only read; they never send commands or change settings.
pub trait Provider: Send + Sync {
    /// Vendor family this provider talks to.
    fn kind(&self) -> ProviderKind;

    /// Current state, one run over all endpoints.
    fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>>;

    /// History the NAS keeps itself since `since` (Unix seconds UTC).
    /// Default: none.
    fn backfill(&self, _since: i64) -> BoxFuture<'_, Result<Vec<Sample>, ProviderError>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    /// Past backup runs since `since` (Unix seconds UTC), where the NAS
    /// reports them. Default: none.
    fn backup_runs(&self, _since: i64) -> BoxFuture<'_, Result<Vec<BackupRun>, ProviderError>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }
}

#[cfg(test)]
mod tests {
    use std::task::{Context, Poll, Waker};

    use super::*;

    struct SnapshotOnly;

    impl Provider for SnapshotOnly {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Qnap
        }

        fn snapshot(&self) -> BoxFuture<'_, Result<Snapshot, ProviderError>> {
            Box::pin(std::future::ready(Err(ProviderError::Unreachable)))
        }
    }

    fn poll_once<T>(mut future: BoxFuture<'_, T>) -> Poll<T> {
        let mut context = Context::from_waker(Waker::noop());
        future.as_mut().poll(&mut context)
    }

    #[test]
    fn default_history_and_runs_are_empty() {
        let provider: Box<dyn Provider> = Box::new(SnapshotOnly);
        assert_eq!(poll_once(provider.backfill(0)), Poll::Ready(Ok(Vec::new())));
        assert_eq!(
            poll_once(provider.backup_runs(0)),
            Poll::Ready(Ok(Vec::new()))
        );
    }

    #[test]
    fn only_login_second_factor_and_certificate_stop_polling() {
        let stopping = [
            ProviderError::LoginFailed,
            ProviderError::SecondFactorRequired,
            ProviderError::CertificateChanged {
                old: "aa".into(),
                new: "bb".into(),
            },
        ];
        let continuing = [
            ProviderError::Unreachable,
            ProviderError::Timeout,
            ProviderError::PermissionDenied,
            ProviderError::Parse("x".into()),
            ProviderError::Http(500),
        ];
        assert!(stopping.iter().all(ProviderError::stops_polling));
        assert!(!continuing.iter().any(ProviderError::stops_polling));
    }
}
