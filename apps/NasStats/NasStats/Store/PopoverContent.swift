import Foundation

/// What the popover of the menu bar symbol shows (#28): which NAS, its
/// latest open event and the footer. Pure functions, so the tests cover
/// them without AppKit.
enum PopoverContent {
    /// Up to this many NAS the head shows a segment each; more go into a menu.
    static let maxSegments = 4

    /// The NAS the popover shows: the chosen one while it is there, else the
    /// first entry (the store sorts the worst first); `nil` without NAS.
    static func shownId(entries: [NasEntry], chosen: String?) -> String? {
        if let chosen, entries.contains(where: { $0.id == chosen }) {
            return chosen
        }
        return entries.first?.id
    }

    /// Latest event of `nasId` that has not ended; at the same start the
    /// smaller `EventNotifier.id(of:)`, so the choice does not flicker.
    static func latestOpenEvent(_ events: [NasEvent], nasId: String) -> NasEvent? {
        events
            .filter { $0.nasId == nasId && $0.endedAt == nil }
            .min { lhs, rhs in
                lhs.startedAt != rhs.startedAt
                    ? lhs.startedAt > rhs.startedAt
                    : EventNotifier.id(of: lhs) < EventNotifier.id(of: rhs)
            }
    }

    /// Line below the event: "offen seit 25 min", like the events tab.
    static func openSince(_ event: NasEvent, now: Date = .now) -> String {
        let seconds = max(0, Int64(now.timeIntervalSince1970) - event.startedAt)
        return "offen seit " + StatusFormat.uptime(UInt64(seconds))
    }

    /// Footer:"Stand vor 2 min" of the shown NAS, "Noch kein Stand"
    /// without one; demo values begin with "Demo · ".
    static func footer(entry: NasEntry?, demo: Bool, now: Date = .now) -> String {
        let stamp = OverviewSummary.stamp(entry.map { [$0] } ?? [], now: now)
        return demo ? "Demo · " + stamp : stamp
    }
}
