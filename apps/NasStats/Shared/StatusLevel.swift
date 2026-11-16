import SwiftUI

/// State of a NAS or one of its parts, as shown by the status dot (#16).
enum StatusLevel: String, Codable, Sendable, CaseIterable {
    case ok
    case warning
    case critical
    case unknown
    /// The NAS did not answer. Kept apart from `unknown` on purpose.
    case offline

    var color: Color {
        switch self {
        case .ok: Color("StatusOk")
        case .warning: Color("StatusWarning")
        case .critical: Color("StatusCritical")
        case .unknown: Color("StatusUnknown")
        case .offline: Color("StatusOffline")
        }
    }

    var label: String {
        switch self {
        case .ok: "Gesund"
        case .warning: "Warnung"
        case .critical: "Kritisch"
        case .unknown: "Unbekannt"
        case .offline: "Nicht erreichbar"
        }
    }
}

/// Fill levels (percent) from which a volume turns yellow and red.
struct FillThresholds: Codable, Sendable, Equatable {
    var warning: Double
    var critical: Double

    static let standard = FillThresholds(warning: 80, critical: 90)

    /// Level for a fill percentage. A missing value is `unknown`.
    func level(for percent: Double?) -> StatusLevel {
        guard let percent else { return .unknown }
        if percent >= critical { return .critical }
        if percent >= warning { return .warning }
        return .ok
    }
}

/// Forecast for a volume to run full.
enum FillForecast: Codable, Sendable, Equatable {
    case noTrend
    case full(days: Double, date: Date?)
}

enum StatusMetrics {
    /// Disk temperature in °C from which the value turns yellow.
    static let diskTempWarningC = 50.0
}

/// Thresholds the user sets in the settings (#21): fill level of a volume
/// for warning and critical, disk temperature for warning. Stored as JSON
/// in the defaults; without FFI types, the widgets share this file.
struct AlertThresholds: Codable, Equatable, Sendable {
    var fill: FillThresholds
    var diskTempWarningC: Double

    static let standard = AlertThresholds(
        fill: .standard, diskTempWarningC: StatusMetrics.diskTempWarningC
    )

    /// Key in the defaults.
    static let defaultsKey = "thresholds"

    /// Allowed ranges of the settings.
    static let fillRange: ClosedRange<Double> = 50...95
    static let diskTempRange: ClosedRange<Double> = 40...65
    /// Smallest gap between warning and critical, one step of the settings.
    static let fillGap = 5.0

    /// Warning at least `fillGap` below critical, both within `fillRange`,
    /// temperature within `diskTempRange`. The steppers of the settings
    /// build their ranges from these values, so they never get empty.
    var isValid: Bool {
        Self.fillRange.contains(fill.warning)
            && Self.fillRange.contains(fill.critical)
            && fill.critical - fill.warning >= Self.fillGap
            && Self.diskTempRange.contains(diskTempWarningC)
    }

    /// The stored thresholds; missing, unreadable or invalid ones give
    /// `.standard`.
    static func load(from defaults: UserDefaults) -> AlertThresholds {
        guard let data = defaults.data(forKey: defaultsKey),
              let stored = try? JSONDecoder().decode(AlertThresholds.self, from: data),
              stored.isValid
        else { return .standard }
        return stored
    }

    func save(to defaults: UserDefaults) {
        guard let data = try? JSONEncoder().encode(self) else { return }
        defaults.set(data, forKey: Self.defaultsKey)
    }
}
