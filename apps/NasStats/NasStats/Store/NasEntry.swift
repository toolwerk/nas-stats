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
    /// Thresholds of the settings for fill level and disk temperature.
    var thresholds: AlertThresholds = .standard

    /// Model of the last snapshot, else of the test query.
    var displayModel: String? { snapshot?.nas.model ?? model }

    /// Time of the last snapshot, Unix seconds UTC.
    var observedAt: Int64? { snapshot?.observedAt }

    var displayName: String {
        name ?? snapshot?.nas.name ?? NasAddress.host(of: baseUrl) ?? baseUrl
    }

    /// Worst level of `findings`, else the health the NAS reports; an
    /// error lifts it to at least warning, a NAS waiting for the user
    /// (block, refused registration) is critical. A NAS silent for hours
    /// never shows green.
    var health: StoreHealth {
        if blocked || problem?.stopsPolling == true || localError != nil {
            return .critical
        }
        let reported = StoreHealth(snapshot?.worstHealth)
        let base = findings.map { StoreHealth($0.level) }.min()
            .map { min($0, reported) } ?? reported
        return problem == nil && queryError == nil ? base : min(base, .warning)
    }

    /// What needs attention, critical first: the findings of the core, but
    /// fill level and disk temperature checked against the thresholds of
    /// the settings (#21) instead of the standard ones of the core, with
    /// the same texts.
    var findings: [Finding] {
        guard let snapshot else { return [] }
        var all = snapshot.findings.filter { $0.kind != .diskTemperature && $0.kind != .volumeFill }
        for disk in snapshot.disks {
            if let temp = disk.temperatureC, temp >= thresholds.diskTempWarningC {
                all.append(Finding(
                    kind: .diskTemperature, level: .warning, subject: disk.id,
                    text: "\(disk.name) bei \(Int(temp.rounded(.down))) °C"
                ))
            }
        }
        for volume in capacityVolumes {
            let level: Health
            switch thresholds.fill.level(for: volume.usedPercent) {
            case .critical: level = .critical
            case .warning: level = .warning
            default: continue
            }
            let percent = Int((volume.usedPercent ?? 0).rounded(.down))
            all.append(Finding(
                kind: .volumeFill, level: level, subject: volume.id,
                text: "\(volume.name) bei \(percent) %"
            ))
        }
        // Order of the rules of the core, critical first; stable within.
        return all.enumerated()
            .sorted { lhs, rhs in
                let l = (lhs.element.level == .critical ? 0 : 1, Self.rank(lhs.element.kind), lhs.offset)
                let r = (rhs.element.level == .critical ? 0 : 1, Self.rank(rhs.element.kind), rhs.offset)
                return l < r
            }
            .map(\.element)
    }

    /// Place of a rule in the order of the core.
    private static func rank(_ kind: FindingKind) -> Int {
        switch kind {
        case .alert: 0
        case .diskHealth, .diskTemperature: 1
        case .volumeHealth: 2
        case .volumeFill: 3
        case .fanHealth: 4
        case .upsOnBattery: 5
        case .backupFailed, .backupOverdue: 6
        }
    }

    /// State for the status dot: "Nicht erreichbar" when the NAS did not
    /// answer and its last state was not critical (a critical NAS stays
    /// red), else from `health`.
    var statusLevel: StatusLevel {
        if let kind = problem?.kind, kind == .unreachable || kind == .timeout,
           health != .critical {
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
    /// A NAS that did not answer or timed out is "Nicht erreichbar", away
    /// from home that is the normal case; the settings show the exact
    /// message.
    var problemLine: String? {
        if let localError { return localError }
        if let kind = problem?.kind, kind == .unreachable || kind == .timeout {
            return "Nicht erreichbar"
        }
        return problem?.message ?? queryError
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
