import Foundation

/// The state of every NAS for the widgets (#23): a small JSON file in the
/// App Group that the app writes after every new state, so the widgets read
/// it without loading the Rust core. No FFI types here, the widget targets
/// compile this file without them.
struct WidgetSnapshot: Codable, Equatable, Sendable {
    /// Format of the file; a file of another version is not read.
    static let currentVersion = 1
    /// Name of the file in the container of the App Group.
    static let fileName = "widget-snapshot.json"

    var schemaVersion: Int = Self.currentVersion
    /// When the app wrote the file.
    var writtenAt: Date
    /// The values come from the demo data set (#22), not from a NAS.
    var demo: Bool
    /// Seconds after `WidgetNas.observedAt` from which a state counts as
    /// old (`staleAfter(intervalMinutes:)`).
    var staleAfterSeconds: Int64
    /// Thresholds of the settings (#21), for the colours of the widgets.
    var thresholds: AlertThresholds
    /// Every NAS in the order of the store, worst first.
    var nas: [WidgetNas]

    /// Seconds after which a state counts as old: 30 minutes, at a longer
    /// query interval the interval plus 5 minutes, so a widget does not
    /// turn grey in every cycle while everything runs normally.
    static func staleAfter(intervalMinutes: Int) -> Int64 {
        max(1800, Int64(intervalMinutes) * 60 + 300)
    }

    /// The state of `nas` is old at `now`. Never in demo mode (screenshots)
    /// and never without a state.
    func isStale(_ nas: WidgetNas, now: Date) -> Bool {
        guard !demo, let observedAt = nas.observedAt else { return false }
        return now.timeIntervalSince(observedAt) > TimeInterval(staleAfterSeconds)
    }

    /// First full second at which the state of `nas` counts as old, for the
    /// timeline of the widgets; `nil` in demo mode or without a state.
    func staleDate(_ nas: WidgetNas) -> Date? {
        guard !demo, let observedAt = nas.observedAt else { return nil }
        return observedAt.addingTimeInterval(TimeInterval(staleAfterSeconds) + 1)
    }

    /// Id of the choice „Automatisch“ in the widget settings (#26). NAS ids
    /// are UUIDs, so it never clashes with one.
    static let automaticId = "automatic"

    /// The NAS a widget shows for its setting `id`: the one with the worst
    /// state (the first) for „Automatisch“, no setting or a NAS that is not
    /// in the snapshot (deleted); otherwise the chosen one. The choice itself
    /// stays stored, a widget cannot change its own setting. `nil` without a
    /// NAS.
    func selected(_ id: String?) -> WidgetNas? {
        guard let id, id != Self.automaticId,
              let chosen = nas.first(where: { $0.id == id })
        else { return nas.first }
        return chosen
    }

    /// Plan of the widget timeline at `now` for the NAS `shown`; see
    /// `timeline(for: [WidgetNas], now:calendar:)`.
    func timeline(
        for shown: WidgetNas?, now: Date, calendar: Calendar = .current
    ) -> (dates: [Date], reloadAfter: Date?) {
        timeline(for: [shown].compactMap { $0 }, now: now, calendar: calendar)
    }

    /// Plan of the widget timeline at `now`: the dates of its entries and
    /// when WidgetKit should ask again. Always an entry for `now`; while the
    /// state of a shown NAS is still current, one for the moment it gets old,
    /// so the widget turns grey without the app. A state that can get old is
    /// asked again after the midnight that follows the last of these moments,
    /// so an old „Stand“ gains its date; otherwise only the app reloads
    /// (`nil`).
    func timeline(
        for shown: [WidgetNas], now: Date, calendar: Calendar = .current
    ) -> (dates: [Date], reloadAfter: Date?) {
        let staleDates = shown.compactMap { staleDate($0) }
        guard let last = staleDates.max() else { return ([now], nil) }
        let upcoming = Set(staleDates.filter { $0 > now }).sorted()
        let midnight = calendar.date(
            byAdding: .day, value: 1, to: calendar.startOfDay(for: max(now, last))
        )
        return ([now] + upcoming, midnight)
    }

    /// The file in the container of the App Group; `nil` without the
    /// container (e.g. in previews or without the entitlement).
    static var fileURL: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
            .appendingPathComponent(fileName)
    }

    /// The snapshot in `url`; `nil` for a missing file, one that does not
    /// decode or one of another `schemaVersion`.
    static func load(from url: URL?) -> WidgetSnapshot? {
        guard let url, let data = try? Data(contentsOf: url),
              let snapshot = try? decoder.decode(WidgetSnapshot.self, from: data),
              snapshot.schemaVersion == currentVersion
        else { return nil }
        return snapshot
    }

    static var encoder: JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        encoder.outputFormatting = .sortedKeys
        return encoder
    }

    static var decoder: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }
}

/// One NAS in the widget snapshot.
struct WidgetNas: Codable, Equatable, Sendable, Identifiable {
    var id: String
    /// Name as the screens show it.
    var name: String
    var model: String?
    /// Short vendor name, e.g. "QNAP".
    var vendor: String?
    var status: StatusLevel
    /// Text of the first finding, critical first.
    var firstFinding: String?
    /// `.warning` or `.critical`, the level of `firstFinding`.
    var firstFindingLevel: StatusLevel?
    var findingCount: Int
    /// Why the NAS has no current state; `nil` without error.
    var problemLine: String?
    /// Volumes whose fill level counts.
    var volumes: [WidgetVolume]
    /// Temperature of the hottest disk in °C.
    var hottestDiskC: Double?
    var cpuPercent: Double?
    var memoryPercent: Double?
    /// Start of the latest backup run.
    var lastBackupAt: Date?
    /// Time of the last snapshot; `nil` before the first answer.
    var observedAt: Date?
    /// Daily fill level of the fullest volume over the last 30 days, oldest
    /// first. Empty without history in the core.
    var fullestVolumeHistory: [WidgetFillPoint]
}

/// One volume in the widget snapshot.
struct WidgetVolume: Codable, Equatable, Sendable, Identifiable {
    var id: String
    var name: String
    var usedPercent: Double?
    var usedBytes: UInt64?
    var totalBytes: UInt64?
    /// Forecast to run full; `nil` without a rising trend or history.
    var forecast: FillForecast?
}

/// Fill level of a volume on one day.
struct WidgetFillPoint: Codable, Equatable, Sendable {
    var day: Date
    var percent: Double
}
