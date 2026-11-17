import Foundation

/// What the detail screen shows of a snapshot, without SwiftUI, so tests
/// cover it (#18).
struct NasDetailContent {
    /// What the backup section shows.
    enum Backups: Equatable {
        /// The tasks of the NAS, in the order of the vendor.
        case tasks([BackupTask])
        /// QNAP without tasks: Hybrid Backup Sync 3 is missing, stopped,
        /// refused or has no backup jobs; the app cannot tell which (#41).
        case noTasks
        /// No section.
        case hidden
    }

    /// Volumes, arrays and shares in the order of the vendor.
    let volumes: [Volume]
    /// Storage pools in the order of the vendor.
    let pools: [Volume]
    let backups: Backups
    /// Stopped containers first, then by name.
    let containers: [Container]
    /// "6 / 7 laufen", nil without containers.
    let containerSummary: String?

    init(snapshot: Snapshot) {
        volumes = snapshot.volumes.filter { $0.kind != .pool }
        pools = snapshot.volumes.filter { $0.kind == .pool }
        if !snapshot.backups.isEmpty {
            backups = .tasks(snapshot.backups)
        } else if snapshot.nas.provider == .qnap {
            backups = .noTasks
        } else {
            backups = .hidden
        }
        containers = snapshot.containers.sorted { lhs, rhs in
            if lhs.running != rhs.running {
                return !lhs.running
            }
            return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
        }
        if snapshot.containers.isEmpty {
            containerSummary = nil
        } else {
            let running = snapshot.containers.filter(\.running).count
            containerSummary = "\(running) / \(snapshot.containers.count) laufen"
        }
    }
}

/// "Gut", "Warnung", "Kritisch"; unknown is "n/a".
func partLabel(_ health: Health?) -> String {
    switch health {
    case .ok: "Gut"
    case .warning: "Warnung"
    case .critical: "Kritisch"
    case nil: StatusFormat.notAvailable
    }
}

/// Colour level of a volume, pool or disk.
func partLevel(_ health: Health?) -> StatusLevel {
    switch health {
    case .ok: .ok
    case .warning: .warning
    case .critical: .critical
    case nil: .unknown
    }
}

/// Name of the vendor feature behind a backup task.
func backupKindLabel(_ kind: BackupKind) -> String {
    switch kind {
    case .hbs3: "Hybrid Backup Sync"
    case .hyperBackup: "Hyper Backup"
    case .activeBackup: "Active Backup"
    case .replication: "Replikation"
    case .cloudSync: "Cloud Sync"
    case .snapshot: "Snapshots"
    case .parity: "Paritätsprüfung"
    }
}
