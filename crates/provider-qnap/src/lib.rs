//! QNAP QTS provider for NAS Stats.
//!
//! QTS answers the CGI endpoints of its web interface with `QDocRoot` XML.
//! This crate logs in over `authLogin.cgi` (user name and Base64 password)
//! and keeps the session id for every further request.
//!
//! Rules:
//!
//! - A session the NAS reports gone (`authPassed` 0, or HTTP 401 on a
//!   data request) leads to exactly one
//!   new login. Is the request still refused after that, the account may
//!   not call this endpoint ([`nas_model::ProviderError::PermissionDenied`]),
//!   without a block.
//! - A login the NAS rejects (wrong password, 2-step verification) blocks
//!   every further login until the user acts, so QTS does not lock the
//!   account or the IP. Network and format errors do not block.
//! - After `logout()` (NAS removed) the client logs in no more and answers
//!   `LoginFailed` without a request, until `login()` or
//!   `set_credentials()`.
//! - The session lives in memory only; after a restart of the app the
//!   client logs in again.
//!
//! [`QnapClient::system`] reads the system values (model, firmware, CPU,
//! memory, temperatures, fans, health, UPS, firmware update),
//! [`QnapClient::network`] the throughput per interface in bytes per second,
//! [`QnapClient::containers`] the containers of the Container Station,
//! [`QnapClient::storage`] the volumes and the disks with their SMART summary
//! and [`QnapClient::backups`] the backup jobs of Hybrid Backup Sync 3 over
//! its REST interface with the QTS session in a header (an empty list when
//! HBS 3 is missing, stopped or refuses; only the status `Finished` is known
//! and counts as a successful run, any other stays unknown).
//! The Container Station has its own login with the same account; without
//! it, without admin rights or after a rejected login the list is empty, and
//! that login is not repeated until the user acts (a missing Container
//! Station is looked for again after [`CONTAINER_RECHECK`]). Storage pools
//! are not read: QTS answers `disk_manage.cgi?store=…` to a GET only with
//! `unknown cmd format.`. [`QnapProvider`] puts system values, storage,
//! network, containers and backups, asked one after the other, into a
//! [`nas_model::Snapshot`] for the core. An HTTP error status of storage,
//! network, containers or backups leaves only that part empty, for backups
//! also HTTP 401, 403 and an answer that is no JSON; other errors fail the
//! snapshot.

mod auth;
mod backups;
mod client;
mod containers;
mod network;
mod provider;
mod storage;
mod system;
mod transport;
mod xml;

pub use auth::SECOND_FACTOR_HINT;
pub use client::QnapClient;
pub use containers::CONTAINER_RECHECK;
pub use provider::QnapProvider;
pub use storage::QnapStorage;
pub use system::{FIRMWARE_INTERVAL, QnapSystem};
pub use transport::Transport;
