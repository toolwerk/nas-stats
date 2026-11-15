import Foundation

/// State of a NAS in the store, worst first.
enum StoreHealth: Int, Comparable, Sendable, CaseIterable {
    case critical
    case warning
    case ok
    case unknown

    static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.rawValue < rhs.rawValue
    }

    var title: String {
        switch self {
        case .critical: "kritisch"
        case .warning: "Warnung"
        case .ok: "ok"
        case .unknown: "unbekannt"
        }
    }
}

/// One NAS as the screens see it: name and address, the last snapshot and
/// what went wrong since.
struct NasEntry: Identifiable, Sendable {
    /// Id of the NAS in the core (`NasAccount.id` as a string, `demo-…` in
    /// demo mode).
    var id: String
    /// Name chosen by the user; `nil` uses the name the NAS reports.
    var name: String?
    /// `https://host[:port]`.
    var baseUrl: String
    /// Model from the test query when the NAS was added.
    var model: String?
    /// Last successful snapshot; it stays after an error.
    var snapshot: Snapshot?
    /// Error of the last query.
    var problem: NasProblem?
    /// Start of the last query, Unix seconds UTC.
    var lastAttemptAt: Int64?
    /// The core does not query this NAS until the user acts.
    var blocked: Bool = false
    /// Why the NAS waits for the user on the app side: the core refused
    /// its registration (`NasAccounts.registrationErrors`).
    var localError: String?
    /// Error thrown by the last query (not a `problem` of the NAS).
    var queryError: String?
    /// Thresholds of the settings for fill level and disk temperature.
    var thresholds: AlertThresholds = .standard

    /// Model of the last snapshot, else of the test query.
    var displayModel: String? { snapshot?.nas.model ?? model }

    /// Time of the last snapshot, Unix seconds UTC.
    var observedAt: Int64? { snapshot?.observedAt }

    var displayName: String {
        name ?? snapshot?.nas.name ?? NasAddress.host(of: baseUrl) ?? baseUrl
    }

    /// Worst health of the snapshot and of the thresholds (fill level of
    /// the volumes, disk temperature); an error lifts it to at least
    /// warning, a NAS waiting for the user (block, refused registration) is
    /// critical. A NAS silent for hours never shows green.
    var health: StoreHealth {
        if blocked || problem?.stopsPolling == true || localError != nil {
            return .critical
        }
        let reported: StoreHealth = switch snapshot?.worstHealth {
        case .critical: .critical
        case .warning: .warning
        case .ok: .ok
        case nil: .unknown
        }
        let base = min(reported, thresholdHealth ?? reported)
        return problem == nil && queryError == nil ? base : min(base, .warning)
    }

    /// Worst level by the thresholds; `nil` when no value crosses one.
    /// Missing values do not count.
    private var thresholdHealth: StoreHealth? {
        guard let snapshot else { return nil }
        var worst: StoreHealth?
        for percent in snapshot.volumes.compactMap(\.usedPercent) {
            switch thresholds.fill.level(for: percent) {
            case .critical: worst = .critical
            case .warning: worst = min(worst ?? .warning, .warning)
            default: break
            }
        }
        if snapshot.disks.contains(where: { ($0.temperatureC ?? -.infinity) >= thresholds.diskTempWarningC }) {
            worst = min(worst ?? .warning, .warning)
        }
        return worst
    }

    /// Worst first, then by name, then by id.
    static func ordered(_ lhs: Self, _ rhs: Self) -> Bool {
        if lhs.health != rhs.health {
            return lhs.health < rhs.health
        }
        switch lhs.displayName.localizedStandardCompare(rhs.displayName) {
        case .orderedAscending: return true
        case .orderedDescending: return false
        case .orderedSame: return lhs.id < rhs.id
        }
    }
}
