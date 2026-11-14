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

#[cfg(test)]
mod tests {
    use super::*;

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
