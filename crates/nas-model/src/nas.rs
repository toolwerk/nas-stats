//! The NAS itself and its system values.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Vendor family of a NAS. Decides which provider crate talks to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Qnap,
    Synology,
    TrueNas,
    Unraid,
}

/// A configured NAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Nas {
    /// Local identifier assigned by the app, not by the vendor.
    pub id: String,
    pub provider: ProviderKind,
    /// Display name as the NAS reports it.
    pub name: Option<String>,
    /// Base URL the provider talks to, e.g. `https://nas.local:5001`.
    pub base_url: String,
    /// Hardware model, e.g. `TS-464`.
    pub model: Option<String>,
    /// Firmware or OS version.
    pub firmware: Option<String>,
    pub hostname: Option<String>,
}

/// System values of a NAS at one point in time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemStatus {
    /// Time since the last boot, in seconds.
    pub uptime_seconds: Option<u64>,
    /// CPU load in percent (0 to 100).
    pub cpu_percent: Option<f64>,
    /// Used memory in percent (0 to 100).
    pub memory_percent: Option<f64>,
    /// Installed memory, in bytes.
    pub memory_total_bytes: Option<u64>,
    /// System temperature in degrees Celsius.
    pub temperature_c: Option<f64>,
    /// CPU temperature in degrees Celsius.
    pub cpu_temperature_c: Option<f64>,
    /// Whether the vendor offers a firmware update.
    pub update_available: Option<bool>,
    /// Version of the offered update, if the vendor names it.
    pub update_version: Option<String>,
}

/// Hashes a disk serial number together with the local NAS id.
///
/// SHA-256 over `nas_id`, a zero byte and the trimmed serial, as 64 lowercase
/// hex characters. The same disk on the same NAS keeps its hash, while the
/// same disk on another NAS gets a different one, so hashes cannot be looked
/// up in precomputed tables or matched across devices. The local id is no
/// secret: whoever holds it can still try candidate serials, so a hash does
/// not belong in data shared outside the app. The plain serial is never stored.
pub fn hash_serial(nas_id: &str, serial: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(nas_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(serial.trim().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest.iter() {
        // Writing into a String cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_serial_is_stable_hex() {
        let first = hash_serial("nas-1", "WD-ABC123");
        assert_eq!(first, hash_serial("nas-1", "  WD-ABC123\n"));
        assert_eq!(first.len(), 64);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn hash_serial_depends_on_nas_id() {
        assert_ne!(
            hash_serial("nas-1", "WD-ABC123"),
            hash_serial("nas-2", "WD-ABC123")
        );
    }

    #[test]
    fn hash_serial_separates_id_and_serial() {
        assert_ne!(hash_serial("nas-1", "23"), hash_serial("nas-12", "3"));
    }
}
