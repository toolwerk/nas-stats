import Foundation

/// Sample values for the widget gallery and the Xcode previews (#24). They
/// are demo values: the snapshots carry `demo: true`.
enum WidgetSamples {
    static let now = Date(timeIntervalSince1970: 1_800_000_000)

    private static func volume(_ name: String, _ percent: Double?, forecast: FillForecast? = nil) -> WidgetVolume {
        WidgetVolume(
            id: name, name: name, usedPercent: percent, usedBytes: nil, totalBytes: nil,
            forecast: forecast)
    }

    private static func nas(
        status: StatusLevel = .warning,
        finding: String? = "Platte 3 bei 52 °C",
        problem: String? = nil,
        volumes: [WidgetVolume],
        observedAgo: TimeInterval? = 60
    ) -> WidgetNas {
        WidgetNas(
            id: "demo-keller", name: "Keller", model: "TS-1685", vendor: "QNAP", status: status,
            firstFinding: finding, firstFindingLevel: finding == nil ? nil : .warning,
            findingCount: finding == nil ? 0 : 1, problemLine: problem, volumes: volumes,
            hottestDiskC: 52, cpuPercent: 9, memoryPercent: 41,
            lastBackupAt: now.addingTimeInterval(-9 * 3600),
            observedAt: observedAgo.map { now.addingTimeInterval(-$0) },
            fullestVolumeHistory: [])
    }

    private static func snapshot(_ nas: WidgetNas, demo: Bool = true) -> WidgetSnapshot {
        WidgetSnapshot(
            writtenAt: now, demo: demo,
            staleAfterSeconds: WidgetSnapshot.staleAfter(intervalMinutes: 5),
            thresholds: .standard, nas: [nas])
    }

    private static let threeVolumes = [
        volume("Archiv", 64), volume("DataVol1", 87, forecast: .full(days: 143, date: nil)),
        volume("Backup", 41),
    ]

    static let demo = snapshot(nas(volumes: threeVolumes))
    static let healthy = snapshot(nas(status: .ok, finding: nil, volumes: threeVolumes))
    static let offline = snapshot(
        nas(status: .offline, finding: nil, problem: "Nicht erreichbar", volumes: threeVolumes,
            observedAgo: 3 * 3600),
        demo: false)
    static let old = snapshot(
        nas(status: .ok, finding: nil, volumes: threeVolumes, observedAgo: 3 * 3600), demo: false)
    static let noVolumes = snapshot(nas(status: .ok, finding: nil, volumes: []))
}
