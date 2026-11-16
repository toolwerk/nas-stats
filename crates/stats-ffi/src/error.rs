//! Errors the apps see as a thrown Swift error.

use nas_model::ProviderError;

use crate::records::ProviderKind;

/// Error of a core call. One variant per `ProviderError`, plus the errors of
/// the core itself.
#[derive(Debug, Clone, PartialEq, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("NAS nicht erreichbar: {detail}")]
    Unreachable { detail: String },
    #[error("Zeitüberschreitung bei der Abfrage")]
    Timeout,
    #[error("Anmeldung fehlgeschlagen")]
    LoginFailed,
    #[error("NAS verlangt einen zweiten Faktor")]
    SecondFactorRequired,
    #[error("Konto hat keine Rechte für diese Abfrage")]
    PermissionDenied,
    #[error("Zertifikat des NAS hat sich geändert (vorher {old}, jetzt {new})")]
    CertificateChanged { old: String, new: String },
    #[error("Unerwartete Antwort: {detail}")]
    Parse { detail: String },
    #[error("HTTP-Status {status}")]
    Http { status: u16 },
    /// No NAS with this id is registered with the core.
    #[error("Kein NAS mit der Kennung {id}")]
    UnknownNas { id: String },
    /// The core cannot talk to this vendor yet.
    #[error("Anbieter {provider:?} wird noch nicht unterstützt")]
    Unsupported { provider: ProviderKind },
    /// An argument of the app is not usable, e.g. a malformed fingerprint or
    /// an `http://` address.
    #[error("Ungültige Angabe: {detail}")]
    InvalidInput { detail: String },
    /// The history archive cannot be opened, read or written.
    #[error("Verlauf nicht lesbar: {detail}")]
    History { detail: String },
    /// A failure inside the core, e.g. a task that panicked.
    #[error("Interner Fehler: {detail}")]
    Internal { detail: String },
}

impl From<stats_core::HistoryError> for FfiError {
    fn from(e: stats_core::HistoryError) -> Self {
        Self::History {
            detail: e.to_string(),
        }
    }
}

impl From<ProviderError> for FfiError {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::Unreachable(detail) => Self::Unreachable { detail },
            ProviderError::Timeout => Self::Timeout,
            ProviderError::LoginFailed => Self::LoginFailed,
            ProviderError::SecondFactorRequired => Self::SecondFactorRequired,
            ProviderError::PermissionDenied => Self::PermissionDenied,
            ProviderError::CertificateChanged { old, new } => Self::CertificateChanged { old, new },
            ProviderError::Parse(detail) => Self::Parse { detail },
            ProviderError::Http(status) => Self::Http { status },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_keep_their_text() {
        let cases = [
            ProviderError::Unreachable("nas:443: refused".into()),
            ProviderError::Timeout,
            ProviderError::LoginFailed,
            ProviderError::SecondFactorRequired,
            ProviderError::PermissionDenied,
            ProviderError::CertificateChanged {
                old: "AA".into(),
                new: "BB".into(),
            },
            ProviderError::Parse("x".into()),
            ProviderError::Http(500),
        ];
        for e in cases {
            assert_eq!(FfiError::from(e.clone()).to_string(), e.to_string());
        }
    }
}
