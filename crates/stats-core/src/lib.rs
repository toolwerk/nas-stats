//! Core of NAS Stats above the providers.
//!
//! For now this is the polling planner: [`Schedule`] holds the rules without
//! a runtime, [`Planner`] runs them on tokio and hands every finished query
//! to a [`PollSink`]. History, events and the callback to the apps attach to
//! that sink.
//!
//! The rules:
//! - Each NAS is queried at most every [`MIN_INTERVAL_SECS`] seconds; a
//!   longer interval may be set, a shorter one counts as the minimum.
//! - On an error the last snapshot stays, with the time and the error.
//! - After a rejected login, a required second factor or a changed
//!   certificate ([`nas_model::ProviderError::stops_polling`]) the NAS stays
//!   blocked until it is unblocked.
//! - After any other error the next attempt comes at the regular time.
//! - A manual refresh waits [`MANUAL_PAUSE_SECS`] seconds after the last
//!   query and moves the next automatic one an interval later.

mod planner;
mod schedule;

pub use planner::{Clock, MAX_SLEEP_SECS, Planner, PollSink, system_clock};
pub use schedule::{MANUAL_PAUSE_SECS, MIN_INTERVAL_SECS, Manual, PollState, Schedule};
