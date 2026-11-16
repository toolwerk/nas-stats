//! Forecast from the demo data set written into a history archive.

use nas_demo::demo;
use stats_core::HistoryStore;

const NOW: i64 = 1_794_665_100;

fn store() -> HistoryStore {
    let set = demo(NOW);
    let mut store = HistoryStore::in_memory().expect("store");
    store.write_samples(&set.samples, NOW).expect("write");
    store
}

#[test]
fn datavol1_is_full_in_143_days() {
    let f = store()
        .forecast("demo-keller", "demo-keller-datavol1", NOW)
        .expect("query")
        .expect("trend");
    // The demo data count from the last value (exactly 143); the line also
    // follows the noise of the 30 days and lands a few hours later. Both the
    // rounded and the truncated value show 143.
    assert_eq!(f.days_to_full.round(), 143.0, "{}", f.days_to_full);
    assert_eq!(f.days_to_full.floor(), 143.0, "{}", f.days_to_full);
    // 2027-04-06 00:00:00 UTC is 1_806_969_600; the day ends 86_400 later.
    assert!(
        (1_806_969_600..1_806_969_600 + 86_400).contains(&f.full_at),
        "{}",
        f.full_at
    );
    assert!(f.growth_per_day > 0.0);
}

#[test]
fn volume_without_trend_or_unknown_gives_none() {
    let store = store();
    assert_eq!(
        store
            .forecast("demo-homelab", "demo-homelab-apps", NOW)
            .expect("query"),
        None
    );
    assert_eq!(
        store
            .forecast("demo-keller", "no-such-volume", NOW)
            .expect("query"),
        None
    );
    assert_eq!(
        store
            .forecast("no-such-nas", "demo-keller-datavol1", NOW)
            .expect("query"),
        None
    );
}
