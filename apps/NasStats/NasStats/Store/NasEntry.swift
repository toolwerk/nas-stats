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

    /// Level of a finding or of the health the NAS reports.
    init(_ health: Health?) {
        switch health {
        case .critical: self = .critical
        case .warning: self = .warning
        case .ok: self = .ok
        case nil: self = .unknown
        }
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
    /// Vendor of the stored NAS, of the snapshot in demo mode; `nil` for a
    /// vendor this version does not know.
    var provider: ProviderKind?
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

    /// Model of the last snapshot, else of the test query.
    var displayModel: String? { snapshot?.nas.model ?? model }

    /// Time of the last snapshot, Unix seconds UTC.
    var observedAt: Int64? { snapshot?.observedAt }

    var displayName: String {
        name ?? snapshot?.nas.name ?? NasAddress.host(of: baseUrl) ?? baseUrl
    }

    /// Worst level of the findings of the core (thresholds included), else
    /// the health the NAS reports; an error lifts it to at least warning,
    /// a NAS waiting for the user (block, refused registration) is
    /// critical. A NAS silent for hours never shows green.
    var health: StoreHealth {
        if blocked || problem?.stopsPolling == true || localError != nil {
            return .critical
        }
        let reported = StoreHealth(snapshot?.worstHealth)
        let base = (snapshot?.findings ?? []).map { StoreHealth($0.level) }.min()
            .map { min($0, reported) } ?? reported
        return problem == nil && queryError == nil ? base : min(base, .warning)
    }

    /// State for the status dot: "Nicht erreichbar" when the NAS did not
    /// answer and nothing blocks it, else from `health`.
    var statusLevel: StatusLevel {
        if let kind = problem?.kind, kind == .unreachable || kind == .timeout,
           !blocked, problem?.stopsPolling != true, localError == nil {
            return .offline
        }
        return switch health {
        case .critical: .critical
        case .warning: .warning
        case .ok: .ok
        case .unknown: .unknown
        }
    }

    /// Short vendor name for the badge of the card.
    var vendorLabel: String? {
        switch provider {
        case .qnap: "QNAP"
        case .synology: "Synology"
        case .trueNas: "TrueNAS"
        case .unraid: "Unraid"
        case nil: nil
        }
    }

    /// Why the NAS has no current state, as one line; `nil` without error.
    var problemLine: String? {
        localError ?? problem?.message ?? queryError
    }

    /// Volumes whose fill level counts, as in the core
    /// (`Snapshot::capacity_volumes`): those of the first kind present in
    /// the order volume, pool, array, share.
    var capacityVolumes: [Volume] {
        let volumes = snapshot?.volumes ?? []
        for kind in [VolumeKind.volume, .pool, .array, .share] {
            let ofKind = volumes.filter { $0.kind == kind }
            if !ofKind.isEmpty {
                return ofKind
            }
        }
        return []
    }

    /// Temperature of the hottest disk in °C.
    var hottestDiskC: Double? {
        snapshot?.disks.compactMap(\.temperatureC).max()
    }

    /// Start of the latest backup run; snapshot tasks and the parity check
    /// are no backup.
    var lastBackupAt: Date? {
        snapshot?.backups
            .filter { $0.kind != .snapshot && $0.kind != .parity }
            .compactMap(\.lastRun)
            .max()
            .map { Date(timeIntervalSince1970: TimeInterval($0)) }
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
