import Foundation
import os

extension WidgetSnapshot {
    /// The widget snapshot of the entries of the store, in their order.
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

    /// The snapshot with the last known state of every NAS that has none
    /// yet: after a launch a NAS without a saved last state (`LastStates`)
    /// stays without one until its interval has passed, a blocked NAS for
    /// good. Their file entry would otherwise lose the last state,
    /// and the widget could never show its age. Name, vendor, error and a
    /// known state come from the new entry; demo and live data never mix.
    func carryingOver(from previous: WidgetSnapshot?) -> WidgetSnapshot {
        guard let previous, !demo, !previous.demo else { return self }
        var result = self
        var carried = false
        for index in result.nas.indices where result.nas[index].observedAt == nil {
            let new = result.nas[index]
            guard var kept = previous.nas.first(where: { $0.id == new.id }),
                  kept.observedAt != nil
            else { continue }
            kept.name = new.name
            kept.model = new.model ?? kept.model
            kept.vendor = new.vendor
            kept.problemLine = new.problemLine
            if new.status != .unknown {
                kept.status = new.status
            }
            result.nas[index] = kept
            carried = true
        }
        if carried {
            // Worst first again, as the store sorts; stable within a level.
            result.nas = result.nas.enumerated()
                .sorted { (Self.rank($0.element.status), $0.offset) < (Self.rank($1.element.status), $1.offset) }
                .map(\.element)
        }
        return result
    }

    /// Place of a state in the order of the store (`StoreHealth`); a NAS
    /// that did not answer counts as a warning there.
    private static func rank(_ level: StatusLevel) -> Int {
        switch level {
        case .critical: 0
        case .warning, .offline: 1
        case .ok: 2
        case .unknown: 3
        }
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
                    forecast: entry.forecast(for: volume.id)
                )
            },
            hottestDiskC: entry.hottestDiskC,
            cpuPercent: entry.snapshot?.system.cpuPercent,
            memoryPercent: entry.snapshot?.system.memoryPercent,
            lastBackupAt: entry.lastBackupAt,
            observedAt: entry.observedAt.map { Date(timeIntervalSince1970: TimeInterval($0)) },
            fullestVolumeHistory: entry.fullestVolumeHistory
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
    /// Last known live state per NAS: the last live snapshot written, else
    /// the file of an earlier launch. Demo snapshots never replace it, so
    /// the demo mode does not lose the state of a blocked NAS.
    private var previous: WidgetSnapshot?

    /// - Parameters:
    ///   - fileURL: Target file; `nil` (no App Group container) writes nothing.
    ///   - reload: Redraws the widgets after a successful write.
    init(fileURL: URL?, reload: @escaping @MainActor () -> Void) {
        self.fileURL = fileURL
        self.reload = reload
        previous = WidgetSnapshot.load(from: fileURL).flatMap { $0.demo ? nil : $0 }
    }

    /// Writes `snapshot` unless it equals the last one written apart from
    /// `writtenAt`; on success the widgets are redrawn. A NAS without a
    /// state keeps its last known one (`carryingOver(from:)`).
    func update(_ new: WidgetSnapshot) {
        let snapshot = new.carryingOver(from: previous)
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
        if !snapshot.demo {
            previous = snapshot
        }
        reload()
    }
}
