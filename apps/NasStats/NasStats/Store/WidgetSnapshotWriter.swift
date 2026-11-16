import Foundation
import os

extension WidgetSnapshot {
    /// The widget snapshot of the entries of the store, in their order.
    /// Forecast and history stay empty until the core delivers them (#67).
    init(
        entries: [NasEntry],
        demo: Bool,
        queryIntervalMinutes: Int,
        thresholds: AlertThresholds,
        now: Date = .now
    ) {
        self.init(
            writtenAt: now,
            demo: demo,
            staleAfterSeconds: Self.staleAfter(intervalMinutes: queryIntervalMinutes),
            thresholds: thresholds,
            nas: entries.map(WidgetNas.init(entry:))
        )
    }
}

extension WidgetNas {
    init(entry: NasEntry) {
        let findings = entry.findings
        let first = findings.first
        let firstLevel: StatusLevel? = switch first?.level {
        case .critical: .critical
        case .warning: .warning
        default: nil
        }
        self.init(
            id: entry.id,
            name: entry.displayName,
            model: entry.displayModel,
            vendor: entry.vendorLabel,
            status: entry.statusLevel,
            firstFinding: first?.text,
            firstFindingLevel: firstLevel,
            findingCount: findings.count,
            problemLine: entry.problemLine,
            volumes: entry.capacityVolumes.map { volume in
                WidgetVolume(
                    id: volume.id,
                    name: volume.name,
                    usedPercent: volume.usedPercent,
                    usedBytes: volume.usedBytes,
                    totalBytes: volume.totalBytes,
                    forecast: nil
                )
            },
            hottestDiskC: entry.hottestDiskC,
            cpuPercent: entry.snapshot?.system.cpuPercent,
            memoryPercent: entry.snapshot?.system.memoryPercent,
            lastBackupAt: entry.lastBackupAt,
            observedAt: entry.observedAt.map { Date(timeIntervalSince1970: TimeInterval($0)) },
            fullestVolumeHistory: []
        )
    }
}

/// Writes the widget snapshot into the App Group and has the widgets
/// redrawn, but only when the content changed: WidgetKit grants a widget a
/// limited number of reloads per day.
@MainActor
final class WidgetSnapshotWriter {
    private static let log = Logger(subsystem: "net.staude.nasstats", category: "widget-snapshot")

    private let fileURL: URL?
    private let reload: @MainActor () -> Void
    /// What was written last; stays after a failed write.
    private var lastWritten: WidgetSnapshot?

    /// - Parameters:
    ///   - fileURL: Target file; `nil` (no App Group container) writes nothing.
    ///   - reload: Redraws the widgets after a successful write.
    init(fileURL: URL?, reload: @escaping @MainActor () -> Void) {
        self.fileURL = fileURL
        self.reload = reload
    }

    /// Writes `snapshot` unless it equals the last one written apart from
    /// `writtenAt`; on success the widgets are redrawn.
    func update(_ snapshot: WidgetSnapshot) {
        if var last = lastWritten {
            last.writtenAt = snapshot.writtenAt
            if last == snapshot { return }
        }
        guard let fileURL else {
            Self.log.error("No App Group container, widget snapshot not written")
            return
        }
        do {
            let data = try WidgetSnapshot.encoder.encode(snapshot)
            try data.write(to: fileURL, options: .atomic)
        } catch {
            Self.log.error("Widget snapshot not written: \(error.localizedDescription, privacy: .public)")
            return
        }
        lastWritten = snapshot
        reload()
    }
}
