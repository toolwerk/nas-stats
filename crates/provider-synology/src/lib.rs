//! Synology DSM provider for NAS Stats.
//!
//! DSM answers its Web API with JSON in an envelope
//! (`{"success": …, "data": …, "error": {"code": …}}`). This crate reads the
//! paths and versions of the APIs from `SYNO.API.Info` and logs in over
//! `SYNO.API.Auth` (DSM 6 or newer, form POST, `format=sid`), then sends the
//! session id as `_sid` with every request.
//!
//! Rules:
//!
//! - DSM gives system values to administrators only (error 105 for other
//!   accounts, see [`ADMIN_HINT`]). Error 105 is
//!   [`nas_model::ProviderError::PermissionDenied`] at once, without a new
//!   login: a new session would not change the rights.
//! - A session the NAS reports gone (106, 107, 119, or HTTP 401 on a data
//!   request) leads to exactly one new login. Is the request still refused
//!   after that, the account may not call it (`PermissionDenied`), without a
//!   block.
//! - A login the NAS rejects blocks every further request until the user
//!   acts, so DSM does not lock the account or the IP: 400, 401, 402 and
//!   unknown codes are `LoginFailed`, 403, 404 and 406 (second factor
//!   missing or rejected) are `SecondFactorRequired`, 407 is `IpBlocked`.
//!   HTTP 401 on the login counts as `LoginFailed`, HTTP 403 and 429 (the
//!   web server refuses or throttles the address) as `IpBlocked`.
//!   Network and format errors do not block.
//! - The second factor is entered once ([`SynologyClient::login_with_code`]);
//!   the NAS then issues a device token, which the caller stores and hands
//!   back. Logins with the token need no code. No TOTP secret is stored.
//! - After `logout()` (NAS removed) the client sends nothing and answers
//!   `LoginFailed`, until `login()`, `login_with_code()` or
//!   `set_credentials()`.
//! - Session and catalog live in memory only; after a restart of the app
//!   the client asks again.
//!
//! The data requests and the [`nas_model::Provider`] follow with #32.

mod api;
mod auth;
mod client;
mod transport;

pub use auth::{ADMIN_HINT, DEVICE_NAME, IP_BLOCKED_HINT, SECOND_FACTOR_HINT};
pub use client::SynologyClient;
pub use transport::Transport;
