//! HTTPS layer shared by all NAS Stats providers.
//!
//! NAS almost always present a self-signed certificate. Instead of switching
//! the check off, trust works in two steps:
//!
//! 1. [`first_contact`] reads the certificate and returns its SHA-256
//!    [`Fingerprint`] without sending any request. The app shows it, the
//!    user confirms it.
//! 2. [`NasClient`] then accepts exactly that certificate. Another one ends
//!    in [`ProviderError::CertificateChanged`] with the old and the new
//!    fingerprint.
//!
//! Decisions:
//!
//! - Only HTTPS. A client cannot be built for `http://`, and there is no
//!   constructor without a fingerprint, so the check cannot be switched off.
//! - A pinned certificate stays valid after it expires and under another
//!   host name: the user confirmed this one certificate.
//! - Every request has a time limit ([`TIMEOUT`], connect
//!   [`CONNECT_TIMEOUT`]), goes out exactly once and follows no redirects.
//! - Each [`NasClient`] keeps the cookies its NAS sets, in memory only.
//!
//! The crate is async; the tokio runtime comes with the FFI crate.

mod client;
mod fingerprint;
mod verifier;

pub use client::{CONNECT_TIMEOUT, NasClient, TIMEOUT, first_contact};
pub use fingerprint::Fingerprint;
pub use nas_model::ProviderError;
