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

/// One NAS as the screens see it: the saved entry, the last snapshot and
/// what went wrong since.
struct NasEntry: Identifiable, Sendable {
    var stored: StoredNas
    /// Last successful snapshot; it stays after an error.
    var snapshot: Snapshot?
    /// Error of the last query.
    var problem: NasProblem?
    /// Start of the last query, Unix seconds UTC.
    var lastAttemptAt: Int64?
    /// The core does not query this NAS until the user acts.
    var blocked: Bool = false
    /// Error on the app side: missing password, refused registration or a
    /// thrown query.
    var localError: String?

    var id: String { stored.id }

    /// Time of the last snapshot, Unix seconds UTC.
    var observedAt: Int64? { snapshot?.observedAt }

    var displayName: String {
        stored.name ?? snapshot?.nas.name ?? URL(string: stored.baseUrl)?.host() ?? stored.baseUrl
    }

    /// Worst health of the snapshot; an error lifts it to at least warning,
    /// a NAS waiting for the user (block, missing password, refused
    /// registration) is critical. A NAS silent for hours never shows green.
    var health: StoreHealth {
        if blocked || problem?.stopsPolling == true || localError != nil {
            return .critical
        }
        let base: StoreHealth = switch snapshot?.worstHealth {
        case .critical: .critical
        case .warning: .warning
        case .ok: .ok
        case nil: .unknown
        }
        return problem == nil ? base : min(base, .warning)
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
