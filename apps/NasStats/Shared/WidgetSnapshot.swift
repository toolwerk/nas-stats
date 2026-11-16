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

    /// Plan of the widget timeline at `now`: the dates of its entries and
    /// when WidgetKit should ask again. Always an entry for `now`; while the
    /// state of the shown (first) NAS is still current, one for the moment
    /// it gets old, so the widget turns grey without the app. A state that
    /// can get old is asked again after the midnight that follows, so an
    /// old „Stand“ gains its date; otherwise only the app reloads (`nil`).
    func timeline(now: Date, calendar: Calendar = .current) -> (dates: [Date], reloadAfter: Date?) {
        guard let first = nas.first, let staleDate = staleDate(first) else {
            return ([now], nil)
        }
        let dates = staleDate > now ? [now, staleDate] : [now]
        let midnight = calendar.date(
            byAdding: .day, value: 1, to: calendar.startOfDay(for: max(now, staleDate))
        )
        return (dates, midnight)
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
    /// first. Empty until the core delivers the history (#67).
    var fullestVolumeHistory: [WidgetFillPoint]
}

/// One volume in the widget snapshot.
struct WidgetVolume: Codable, Equatable, Sendable, Identifiable {
    var id: String
    var name: String
    var usedPercent: Double?
    var usedBytes: UInt64?
    var totalBytes: UInt64?
    /// Forecast to run full; `nil` until the core delivers it (#67).
    var forecast: FillForecast?
}

/// Fill level of a volume on one day.
struct WidgetFillPoint: Codable, Equatable, Sendable {
    var day: Date
    var percent: Double
}
