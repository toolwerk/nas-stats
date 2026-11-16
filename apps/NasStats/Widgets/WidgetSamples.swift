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

    // MARK: Widget „Alle NAS“ (#38)

    private static let tb: UInt64 = 1_099_511_627_776

    private static func sizedVolume(_ name: String, percent: Double, totalTB: Double) -> WidgetVolume {
        let total = UInt64(totalTB * Double(tb))
        return WidgetVolume(
            id: name, name: name, usedPercent: percent,
            usedBytes: UInt64(Double(total) * percent / 100), totalBytes: total, forecast: nil)
    }

    private static func row(
        _ id: String, _ name: String, status: StatusLevel, findings: Int = 0,
        volumes: [WidgetVolume], history: [WidgetFillPoint] = []
    ) -> WidgetNas {
        WidgetNas(
            id: id, name: name, model: nil, vendor: nil, status: status,
            firstFinding: findings > 0 ? "Platte 3 bei 52 °C" : nil,
            firstFindingLevel: findings > 0 ? .warning : nil, findingCount: findings,
            problemLine: nil, volumes: volumes, hottestDiskC: nil, cpuPercent: nil,
            memoryPercent: nil, lastBackupAt: now.addingTimeInterval(-9 * 3600),
            observedAt: now.addingTimeInterval(-60), fullestVolumeHistory: history)
    }

    /// 30 daily values from 84 to 88 percent, the last one today.
    private static let thirtyDays: [WidgetFillPoint] = (0 ..< 30).map { offset in
        WidgetFillPoint(
            day: now.addingTimeInterval(-Double(29 - offset) * 86400),
            percent: 84 + 4 * Double(offset) / 29)
    }

    private static func allSnapshot(history: [WidgetFillPoint]) -> WidgetSnapshot {
        WidgetSnapshot(
            writtenAt: now, demo: true,
            staleAfterSeconds: WidgetSnapshot.staleAfter(intervalMinutes: 5),
            thresholds: .standard,
            nas: [
                row("demo-keller", "Keller", status: .warning, findings: 1,
                    volumes: [sizedVolume("DataVol1", percent: 87, totalTB: 21.8)], history: history),
                row("demo-buero", "Büro", status: .ok,
                    volumes: [sizedVolume("Volume 1", percent: 52, totalTB: 10.9)]),
                row("demo-homelab", "Homelab", status: .ok,
                    volumes: [sizedVolume("tank", percent: 64, totalTB: 21.8),
                              sizedVolume("apps", percent: 23, totalTB: 0.7)]),
                row("demo-archiv", "Archiv", status: .ok,
                    volumes: [sizedVolume("Archiv", percent: 30, totalTB: 8)]),
            ])
    }

    static let all = allSnapshot(history: thirtyDays)
    static let allWithoutHistory = allSnapshot(history: [])
}
