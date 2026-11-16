//! What a finished query leaves in the history archive. The app (through
//! the FFI core) and the collector on the NAS (#46) write it the same way.

use nas_model as model;

use crate::history::{EventRecord, HistoryError, HistoryStore};
use crate::schedule::PollState;

/// Writes one finished query: after a success the snapshot and the events
/// it opens, raises or closes by `thresholds`; after no answer
/// (unreachable, time-out, HTTP 502/503/504 of a proxy) an "unreachable"
/// event when the last snapshot is 15 minutes old. Without any snapshot
/// nothing.
pub fn archive_poll(
    store: &mut HistoryStore,
    state: &PollState,
    thresholds: &model::Thresholds,
    now: i64,
) -> Result<(), HistoryError> {
    let Some(snapshot) = &state.snapshot else {
        return Ok(());
    };
    let changes = if let Some(error) = &state.error {
        // Only a NAS that gave no answer is unreachable (no connection,
        // time-out, HTTP 502/503/504 of a proxy); a refused login or an odd
        // answer still came from it.
        if !error.is_no_answer() {
            return Ok(());
        }
        let open = open_events(store, &state.nas_id)?;
        model::detect_unreachable(&open, &snapshot.nas, Some(snapshot.observed_at), now)
            .into_iter()
            .collect()
    } else {
        store.write_snapshot(snapshot, now)?;
        let open = open_events(store, &state.nas_id)?;
        model::detect_events(&open, snapshot, thresholds)
    };
    let records: Vec<EventRecord> = changes
        .iter()
        .map(|change| EventRecord::from(change.event()))
        .collect();
    store.write_events(&records)
}

/// Open events of a NAS; kinds of a newer build are left out.
fn open_events(store: &HistoryStore, nas_id: &str) -> Result<Vec<model::Event>, HistoryError> {
    Ok(store
        .open_events(nas_id)?
        .iter()
        .filter_map(|record| record.to_event().ok())
        .collect())
}
