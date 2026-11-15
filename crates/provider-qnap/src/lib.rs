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
//! memory, temperatures, fans, health, firmware update). Volumes, disks and
//! the network follow in later issues.

mod auth;
mod client;
mod system;
mod transport;
mod xml;

pub use auth::SECOND_FACTOR_HINT;
pub use client::QnapClient;
pub use system::{FIRMWARE_INTERVAL, QnapSystem};
pub use transport::Transport;
