//! Unraid provider for NAS Stats.
//!
//! Unraid 7.2 and newer answer a GraphQL API at `https://<host>/graphql`.
//! This crate sends every query with an API key in the header `x-api-key`;
//! the role `VIEWER` reads everything it asks for. There is no session and
//! no login.
//!
//! Rules:
//!
//! - Queries use only fields that already exist in the API of Unraid 7.2.0
//!   (API 4.25): GraphQL refuses a whole query with one unknown field.
//! - Disks come from `array` only. The query `disks` (SMART, serial number)
//!   wakes sleeping array disks (unraid/api#2018), so it is never sent; the
//!   SMART text and the power-on hours stay `None`.
//! - Every field is read tolerantly: a missing field, `null`, a number sent
//!   as text or an unknown enum value leaves only that value `None`.
//! - A refused key (`UNAUTHENTICATED`, HTTP 401) is
//!   [`nas_model::ProviderError::LoginFailed`], which stops the planner
//!   until the user acts. Missing rights (`FORBIDDEN`, HTTP 403) are
//!   [`nas_model::ProviderError::PermissionDenied`].
//! - Capacities come in kilobytes of 1000 bytes.
//! - The array disk ids hold model and serial number. They are only hashed
//!   ([`nas_model::hash_serial`]), never kept or logged in plain text.
//!
//! [`UnraidProvider`] asks the required part (server, array, disks) and then
//! metrics, shares, notifications, UPS and containers one after the other,
//! each in a query of its own, so a missing Docker service or UPS leaves
//! only that part empty. [`UnraidProvider::backup_runs`] reads the parity
//! history; the API keeps no history of measured values, so there is no
//! backfill.

mod client;
mod map;
mod provider;
mod queries;
mod transport;
mod wire;

pub use client::UnraidClient;
pub use map::PARITY_TASK_ID;
pub use provider::UnraidProvider;
pub use transport::Transport;
