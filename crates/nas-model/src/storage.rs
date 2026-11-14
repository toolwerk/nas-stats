//! Volumes, pools and disks.

use serde::{Deserialize, Serialize};

use crate::health::{Health, percent};

/// What a [`Volume`] stands for at the vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeKind {
    /// A volume (Synology, QNAP).
    Volume,
    /// A storage pool (QNAP, Synology) or ZFS pool (TrueNAS).
    Pool,
    /// The Unraid array.
    Array,
    /// A share (Unraid) or dataset.
    Share,
}

/// A storage volume, pool, array or share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub kind: VolumeKind,
    /// Id of the pool this volume lives in, when the vendor reports it.
    pub pool_id: Option<String>,
    /// Used space in bytes.
    pub used_bytes: Option<u64>,
    /// Total space in bytes.
    pub total_bytes: Option<u64>,
    pub health: Option<Health>,
    /// Status text exactly as the vendor reports it.
    pub status_text: Option<String>,
    /// RAID level as the vendor names it, e.g. `RAID 5`, `raidz1`.
    pub raid: Option<String>,
    pub scrub: Option<ScrubStatus>,
}

impl Volume {
    /// Used share in percent, `None` when used or total space is unknown or the total is zero.
    pub fn used_percent(&self) -> Option<f64> {
        percent(self.used_bytes?, self.total_bytes?)
    }
}

/// State of a scrub, parity check or RAID scrubbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrubState {
    Running,
    Paused,
    Finished,
    Canceled,
    Failed,
}

/// Last or current scrub of a volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrubStatus {
    pub state: Option<ScrubState>,
    /// End of the last finished scrub, Unix seconds UTC.
    pub last_finished_at: Option<i64>,
    /// Errors found by the last scrub.
    pub errors: Option<u64>,
    /// Progress of a running scrub in percent (0 to 100).
    pub progress_percent: Option<f64>,
}

/// A physical disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    /// Serial number hashed with [`crate::hash_serial`]; the plain serial is never kept.
    pub serial_hash: Option<String>,
    /// Disk temperature in degrees Celsius.
    pub temperature_c: Option<f64>,
    pub health: Option<Health>,
    /// SMART summary exactly as the vendor reports it.
    pub smart_text: Option<String>,
    /// Power-on time in hours.
    pub power_on_hours: Option<u64>,
    /// Whether the disk spins right now; `Some(false)` means standby.
    pub spinning: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(used: Option<u64>, total: Option<u64>) -> Volume {
        Volume {
            id: "1".into(),
            name: "Volume 1".into(),
            kind: VolumeKind::Volume,
            pool_id: None,
            used_bytes: used,
            total_bytes: total,
            health: Some(Health::Ok),
            status_text: None,
            raid: None,
            scrub: None,
        }
    }

    #[test]
    fn used_percent_of_a_volume() {
        assert_eq!(volume(Some(250), Some(1000)).used_percent(), Some(25.0));
    }

    #[test]
    fn used_percent_without_total_is_none() {
        assert_eq!(volume(Some(250), Some(0)).used_percent(), None);
        assert_eq!(volume(Some(250), None).used_percent(), None);
        assert_eq!(volume(None, Some(1000)).used_percent(), None);
    }
}
