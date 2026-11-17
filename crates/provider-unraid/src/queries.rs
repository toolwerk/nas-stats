//! The GraphQL queries. Only fields of the API of Unraid 7.2.0 (API 4.25):
//! one unknown field makes GraphQL refuse the whole query.
//!
//! Never asked: `disks` (wakes sleeping array disks, unraid/api#2018) and
//! `docker.organizer`.

/// Server, array and its disks. Without them there is no snapshot.
pub(crate) const MAIN: &str = "query { \
vars { name } \
info { os { hostname uptime } system { manufacturer model } versions { core { unraid } } } \
array { state capacity { kilobytes { free used total } } \
parityCheckStatus { date duration status progress } \
parities { id name device status temp numErrors isSpinning } \
disks { id name device status temp numErrors isSpinning } \
caches { id name device status temp numErrors isSpinning } } \
}";

/// CPU load and memory.
pub(crate) const METRICS: &str =
    "query { metrics { cpu { percentTotal } memory { total percentTotal } } }";

/// User shares with their space in kilobytes.
pub(crate) const SHARES: &str = "query { shares { name free used } }";

/// Unread notifications, at most 50.
pub(crate) const NOTIFICATIONS: &str = "query { notifications { \
list(filter: {type: UNREAD, offset: 0, limit: 50}) { title subject importance timestamp } } }";

/// UPS devices of the built-in UPS support.
pub(crate) const UPS: &str =
    "query { upsDevices { status battery { chargeLevel estimatedRuntime } } }";

/// Docker containers.
pub(crate) const DOCKER: &str = "query { docker { containers { id names state } } }";

/// Past parity checks from the parity log.
pub(crate) const PARITY_HISTORY: &str = "query { parityHistory { date duration status errors } }";
