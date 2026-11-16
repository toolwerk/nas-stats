//! Statistics core of NAS Stats.
//!
//! No vendor reports the history of its values, so the app keeps its own
//! archive in SQLite ([`HistoryStore`]). Every snapshot is stored as samples,
//! rolled up into hourly and daily values and pruned after fixed periods.
//! Hours and days are counted in UTC.
//!
//! The polling planner decides when each NAS is queried: [`Schedule`] holds
//! the rules without a runtime, [`Planner`] runs them on tokio and hands
//! every finished query to a [`PollSink`]. History, events and the callback
//! to the apps attach to that sink.
//!
//! The rules of the planner:
//! - Each NAS is queried at most every [`MIN_INTERVAL_SECS`] seconds; a
//!   longer interval may be set, a shorter one counts as the minimum.
//! - On an error the last snapshot stays, with the time and the error.
//! - After a rejected login, a required second factor or a changed
//!   certificate ([`nas_model::ProviderError::stops_polling`]) the NAS stays
//!   blocked until it is unblocked.
//! - After any other error the next attempt comes at the regular time.
//! - A manual refresh waits [`MANUAL_PAUSE_SECS`] seconds after the last
//!   query and moves the next automatic one an interval later.

mod history;
mod planner;
mod schedule;

pub use history::{
    EventRecord, HistoryError, HistoryStore, Point, Range, Resolution, SCHEMA_VERSION, Series,
};
pub use planner::{Clock, MAX_SLEEP_SECS, Planner, PollSink, system_clock};
pub use schedule::{MANUAL_PAUSE_SECS, MIN_INTERVAL_SECS, Manual, PollState, Schedule};
