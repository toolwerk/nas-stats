//! Demo events as in the mockup: two open (Platte 3 too hot, DataVol1 over
//! the threshold) and two resolved (UPS on battery for 4 minutes, Homelab
//! unreachable for 18 minutes).
//!
//! "Backup erfolgreich" and "Firmware-Update" of the mockup are missing, as no
//! rule of the core knows them.

use nas_model::{
    Event, EventChange, EventKind, Health, Snapshot, Thresholds, detect_events, detect_unreachable,
};

const MINUTE: i64 = 60;

/// Start of the open Platte 3 event, seconds before `now` (13:40 at 14:05).
pub(crate) const DISK3_SINCE: i64 = 25 * MINUTE;
/// Start of the open DataVol1 event, seconds before `now` (22:15 yesterday).
pub(crate) const DATAVOL1_SINCE: i64 = 15 * 3600 + 50 * MINUTE;
/// Start of the UPS event, seconds before `now` (06:31 yesterday).
pub(crate) const UPS_SINCE: i64 = 31 * 3600 + 34 * MINUTE;
/// Time on battery.
pub(crate) const UPS_DURATION: i64 = 4 * MINUTE;
/// Last snapshot of Homelab before it was gone, seconds before `now` (03:10
/// yesterday).
pub(crate) const HOMELAB_SINCE: i64 = 34 * 3600 + 55 * MINUTE;
/// Time Homelab was gone.
pub(crate) const HOMELAB_DURATION: i64 = 18 * MINUTE;

/// The demo events for `now`, newest first.
pub(crate) fn events(now: i64, snapshots: &[Snapshot]) -> Vec<Event> {
    let find = |id: &str| snapshots.iter().find(|s| s.nas.id == id);
    let mut events = Vec::new();

    if let Some(keller) = find("demo-keller") {
        for change in detect_events(&[], keller, &Thresholds::default()) {
            let EventChange::Opened(mut event) = change else {
                continue;
            };
            event.started_at = match event.kind {
                EventKind::DiskTemperature => now - DISK3_SINCE,
                EventKind::VolumeFill => now - DATAVOL1_SINCE,
                _ => event.started_at,
            };
            events.push(event);
        }
        events.push(Event {
            nas_id: keller.nas.id.clone(),
            kind: EventKind::UpsOnBattery,
            subject: None,
            level: Health::Critical,
            text: "USV auf Batterie".into(),
            started_at: now - UPS_SINCE,
            ended_at: Some(now - UPS_SINCE + UPS_DURATION),
        });
    }

    if let Some(homelab) = find("demo-homelab") {
        let last_seen = now - HOMELAB_SINCE;
        let gone = detect_unreachable(
            &[],
            &homelab.nas,
            Some(last_seen),
            last_seen + HOMELAB_DURATION,
        );
        if let Some(EventChange::Opened(mut event)) = gone {
            event.ended_at = Some(last_seen + HOMELAB_DURATION);
            events.push(event);
        }
    }

    events.sort_by_key(|e| std::cmp::Reverse(e.started_at));
    events
}
