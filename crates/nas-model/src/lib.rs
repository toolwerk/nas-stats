//! Shared data model for NAS Stats.
//!
//! Providers translate vendor answers into these types. A value the vendor
//! does not report stays `None`; nothing is guessed.
//!
//! Units are fixed: times are Unix seconds in UTC, sizes are bytes,
//! temperatures are degrees Celsius. Each field names its unit.

mod backup;
mod device;
mod findings;
mod health;
mod nas;
mod provider;
mod sample;
mod snapshot;
mod storage;

pub use backup::{BackupKind, BackupResult, BackupRun, BackupTask};
pub use device::{Alert, AlertLevel, Container, Fan, NetInterface, Ups, UpsState};
pub use findings::{Finding, FindingKind, Thresholds};
pub use health::{Health, percent};
pub use nas::{Nas, ProviderKind, SystemStatus, hash_serial};
pub use provider::{BoxFuture, Provider, ProviderError};
pub use sample::{Metric, Sample};
pub use snapshot::Snapshot;
pub use storage::{Disk, ScrubState, ScrubStatus, Volume, VolumeKind};
