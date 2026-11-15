import SwiftUI

/// State of a NAS or one of its parts, as shown by the status dot (#16).
enum StatusLevel: Sendable, CaseIterable {
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
struct FillThresholds: Sendable, Equatable {
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
enum FillForecast: Sendable, Equatable {
    case noTrend
    case full(days: Double, date: Date?)
}

enum StatusMetrics {
    /// Disk temperature in °C from which the value turns yellow.
    static let diskTempWarningC = 50.0
}
