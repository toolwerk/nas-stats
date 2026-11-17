//! TrueNAS (SCALE and Community Edition 25.04 and later) for NAS Stats.
//!
//! TrueNAS is read over JSON-RPC 2.0 on a WebSocket,
//! `wss://<host>/api/current`. The REST API is not used: it goes away with
//! TrueNAS 27. This crate builds the connection and the login; the values
//! of pools, disks and alerts follow in #34.
//!
//! Rules:
//!
//! - Only TLS with the confirmed certificate. The connection runs over
//!   [`nas_http::connect_pinned`]; another certificate is
//!   [`ProviderError::CertificateChanged`]. `http://` and `ws://` are refused
//!   before any connection, because TrueNAS revokes an API key that arrives
//!   without TLS.
//! - Access is user name and API key ([`TruenasCredentials`]); TrueNAS needs
//!   the user for every login. Advice for the user: [`ROLE_HINT`], a user of
//!   its own with the role `READONLY_ADMIN`.
//! - Login method: when `auth.mechanism_choices` offers `SCRAM` (TrueNAS 26
//!   and later), SCRAM-SHA-512, so the key never goes over the wire, bound to
//!   the certificate (`tls-server-end-point`) when its signature algorithm
//!   allows. Otherwise, also when the method fails, the key goes in plain
//!   text over TLS (`API_KEY_PLAIN`, TrueNAS 25). The official client
//!   forbids this fallback against a man in the middle; here the pinned
//!   certificate keeps him out, and without it TrueNAS 25 would not work.
//! - A rejected key, an expired one, a user without API access, a wrong
//!   server signature in SCRAM ([`ProviderError::LoginFailed`]) and a
//!   required second factor ([`ProviderError::SecondFactorRequired`]) block
//!   the client until [`TruenasClient::set_credentials`], without any
//!   further connection. A redirect to the other controller is
//!   [`ProviderError::Unreachable`] and does not block.
//! - Every [`TruenasClient::session`] is a connection of its own; the SCRAM
//!   keys derived from the key stay in memory per salt and iteration count.
//! - [`Session::call`] waits [`nas_http::TIMEOUT`] per call, matches answers
//!   by `id` and skips events. Error objects map to
//!   [`ProviderError::PermissionDenied`] (`EACCES`), [`ProviderError::Http`]
//!   404 (method unknown) or [`ProviderError::Parse`] with the reason.
//! - Only reading: the crate sends no command and changes no setting.

mod client;
mod credentials;
mod rpc;
mod scram;

pub use client::TruenasClient;
pub use credentials::{ROLE_HINT, TruenasCredentials};
pub use nas_model::ProviderError;
pub use rpc::Session;
