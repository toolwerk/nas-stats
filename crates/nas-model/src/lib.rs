//! Shared data model for NAS Stats.
//!
//! Providers translate vendor answers into these types. A value the vendor
//! does not report stays `None`; nothing is guessed.

use serde::{Deserialize, Serialize};

/// Health as the vendor reports it, reduced to three levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Health {
    Ok,
    Warning,
    Critical,
}

/// A storage volume or pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub health: Health,
}

impl Volume {
    /// Used share in percent, `None` when the total is unknown (zero).
    pub fn used_percent(&self) -> Option<f64> {
        if self.total_bytes == 0 {
            return None;
        }
        Some(self.used_bytes as f64 / self.total_bytes as f64 * 100.0)
    }
}

/// Why a request to a NAS failed. The `String` fields carry a detail for the
/// log; the message shown to the user comes from the variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NasError {
    /// No connection: host unknown, port closed, TLS handshake failed.
    #[error("NAS nicht erreichbar: {0}")]
    Unreachable(String),
    /// The NAS did not answer within the time limit.
    #[error("Zeitüberschreitung bei der Anfrage an das NAS")]
    Timeout,
    /// User name or password rejected.
    #[error("Anmeldung am NAS fehlgeschlagen")]
    LoginFailed,
    /// Logged in, but the account may not read this.
    #[error("Keine Berechtigung für diese Abfrage")]
    PermissionDenied,
    /// The NAS presents another certificate than the confirmed one.
    /// Both values are SHA-256 fingerprints as `AB:CD:…`.
    #[error("Zertifikat des NAS hat sich geändert (bestätigt {old}, jetzt {new})")]
    CertificateChanged { old: String, new: String },
    /// An answer or input could not be read.
    #[error("Antwort nicht lesbar: {0}")]
    Parse(String),
    /// Any other HTTP status outside 2xx.
    #[error("NAS antwortet mit HTTP-Status {0}")]
    Http(u16),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_german() {
        assert_eq!(
            NasError::Http(500).to_string(),
            "NAS antwortet mit HTTP-Status 500"
        );
        let changed = NasError::CertificateChanged {
            old: "AA".into(),
            new: "BB".into(),
        };
        assert!(changed.to_string().contains("bestätigt AA, jetzt BB"));
    }

    fn volume(used: u64, total: u64) -> Volume {
        Volume {
            id: "1".into(),
            name: "Volume 1".into(),
            used_bytes: used,
            total_bytes: total,
            health: Health::Ok,
        }
    }

    #[test]
    fn used_percent_of_a_volume() {
        assert_eq!(volume(250, 1000).used_percent(), Some(25.0));
    }

    #[test]
    fn used_percent_without_total_is_none() {
        assert_eq!(volume(250, 0).used_percent(), None);
    }
}
