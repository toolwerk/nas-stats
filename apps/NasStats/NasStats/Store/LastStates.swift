import Foundation
import os

/// The last snapshot of every NAS as the core saved it
/// (`NasCore.savedState`), in a file of the App Group, so the app shows it
/// after a restart, e.g. away from home without a connection to the NAS.
struct LastStates {
    private static let log = Logger(subsystem: "net.staude.nasstats", category: "last-states")

    /// Target file; `nil` reads and writes nothing (tests, previews).
    let fileURL: URL?

    /// `last-states.json` in the container of the App Group; `nil` without
    /// the container.
    static var liveURL: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
            .appendingPathComponent("last-states.json")
    }

    /// NAS id → JSON of the core; empty for a missing or unreadable file.
    func load() -> [String: String] {
        guard let fileURL, let data = try? Data(contentsOf: fileURL),
              let states = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return states
    }

    /// Replaces the file; a failure is only logged, the app goes on with
    /// the states in memory.
    func save(_ states: [String: String]) {
        guard let fileURL else { return }
        do {
            let data = try JSONEncoder().encode(states)
            try data.write(to: fileURL, options: .atomic)
        } catch {
            Self.log.error("Last states not saved: \(String(describing: error), privacy: .public)")
        }
    }
}
