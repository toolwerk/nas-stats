//! Statistics core of NAS Stats.
//!
//! No vendor reports the history of its values, so the app keeps its own
//! archive in SQLite ([`HistoryStore`]). Every snapshot is stored as samples,
//! rolled up into hourly and daily values and pruned after fixed periods.
//! Hours and days are counted in UTC.

mod history;

pub use history::{HistoryError, HistoryStore, Point, Range, Resolution, SCHEMA_VERSION, Series};
