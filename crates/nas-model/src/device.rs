//! UPS, fans, containers, alerts and network interfaces.

use serde::{Deserialize, Serialize};

use crate::health::Health;

/// Power state of a UPS. An unknown state is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpsState {
    Online,
    OnBattery,
}

/// The UPS attached to a NAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ups {
    pub state: Option<UpsState>,
    /// Battery charge in percent (0 to 100).
    pub charge_percent: Option<f64>,
    /// Remaining runtime on battery, in seconds.
    pub runtime_seconds: Option<u64>,
}

/// A fan of the NAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fan {
    pub name: String,
    /// Speed in revolutions per minute.
    pub rpm: Option<u32>,
    pub health: Option<Health>,
}

/// A container or app running on the NAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Container {
    pub name: String,
    pub running: bool,
}

/// Severity of an [`Alert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

/// An alert or notification the NAS reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub level: AlertLevel,
    /// Text exactly as the vendor reports it.
    pub text: String,
    /// When the alert was raised, Unix seconds UTC.
    pub since: Option<i64>,
}

/// Throughput of a network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetInterface {
    pub name: String,
    /// Received bytes per second.
    pub rx_bytes_per_sec: Option<f64>,
    /// Sent bytes per second.
    pub tx_bytes_per_sec: Option<f64>,
}
