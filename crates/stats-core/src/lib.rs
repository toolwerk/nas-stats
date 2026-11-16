//! Statistics core of NAS Stats.
//!
//! No vendor reports the history of its values, so the app keeps its own
//! archive in SQLite ([`HistoryStore`]). Every snapshot is stored as samples,
//! rolled up into hourly and daily values and pruned after fixed periods.
//! Hours and days are counted in UTC.
//!
//! The forecast "full in X days" ([`forecast`], [`HistoryStore::forecast`])
//! runs a least-squares line through the daily values of the used space of
//! the last 30 days:
//! - It needs at least 7 daily values in those days (gaps do not count, the
//!   line uses the real distances) and a rising line, otherwise there is no
//!   trend (`None`). A volume that would be full only after more than 10
//!   years has no trend either.
//! - The days are counted from the point of the line at today, not from the
//!   last value, so a large cleanup does not make the forecast jump.
//! - A line already above the size of the volume gives 0 days ("full today").
//!
//! [`archive_poll`] writes a finished query into the archive, with the
//! events it opens or closes; the app and the collector on the NAS (#46)
//! share it. The collector hands its history out as an [`Export`]
//! ([`HistoryStore::export`]), the app takes it over with
//! [`HistoryStore::import`].
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

mod archive;
mod export;
mod forecast;
mod history;
mod planner;
mod schedule;

pub use archive::archive_poll;
pub use export::{
    CollectorNas, EXPORT_VERSION, Export, ExportBackupRun, ExportBucket, ExportEvent, ExportSample,
    ImportCount,
};
pub use forecast::{FORECAST_DAYS, Forecast, MAX_FORECAST_DAYS, MIN_FORECAST_DAYS, forecast};
pub use history::{
    EventRecord, HistoryError, HistoryStore, Point, Range, Resolution, SCHEMA_VERSION, Series,
};
pub use planner::{Clock, MAX_SLEEP_SECS, Planner, PollSink, system_clock};
pub use schedule::{MANUAL_PAUSE_SECS, MIN_INTERVAL_SECS, Manual, PollState, Schedule};
