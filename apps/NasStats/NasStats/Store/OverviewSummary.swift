import Foundation

/// Texts of the head of the overview (#17). Pure functions, so the tests
/// cover them without rendering a view.
enum OverviewSummary {
    /// State of all NAS in one sentence: "Alle 3 NAS gesund",
    /// "3 NAS: 1 mit Warnung, 2 gesund"; demo values begin with
    /// "Demo-Daten · ".
    static func sentence(_ entries: [NasEntry], demo: Bool) -> String {
        let prefix = demo ? "Demo-Daten · " : ""
        return prefix + state(of: entries)
    }

    /// Age of the oldest state: "Stand vor 2 min"; "Noch kein Stand"
    /// while no NAS has a snapshot.
    static func stamp(_ entries: [NasEntry], now: Date = .now) -> String {
        guard let oldest = entries.compactMap(\.observedAt).min() else {
            return "Noch kein Stand"
        }
        return "Stand " + StatusFormat.age(Date(timeIntervalSince1970: TimeInterval(oldest)), now: now)
    }

    private static func state(of entries: [NasEntry]) -> String {
        guard !entries.isEmpty else { return "Noch kein NAS" }
        let levels = entries.map(\.statusLevel)
        if levels.allSatisfy({ $0 == .ok }) {
            return entries.count == 1 ? "1 NAS gesund" : "Alle \(entries.count) NAS gesund"
        }
        let parts: [(StatusLevel, String)] = [
            (.critical, "kritisch"),
            (.warning, "mit Warnung"),
            (.offline, "nicht erreichbar"),
            (.unknown, "unbekannt"),
            (.ok, "gesund"),
        ]
        let counted = parts.compactMap { level, text -> String? in
            let count = levels.count { $0 == level }
            return count == 0 ? nil : "\(count) \(text)"
        }
        return "\(entries.count) NAS: " + counted.joined(separator: ", ")
    }
}
