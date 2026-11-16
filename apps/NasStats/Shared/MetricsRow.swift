import SwiftUI

/// Row of key figures: hottest disk, CPU, RAM, UPS, last backup (#16).
/// Missing values show "n/a".
struct MetricsRow: View {
    var diskTempC: Double?
    var cpuPercent: Double?
    var ramPercent: Double?
    var upsChargePercent: Double?
    var upsOnBattery = false
    var lastBackup: Date?
    var now: Date = .now
    /// From this disk temperature on the value turns yellow (settings, #21).
    var diskTempWarningC: Double = StatusMetrics.diskTempWarningC

    @Environment(\.locale) private var locale

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            cell("Platten", StatusFormat.temperature(diskTempC, locale: locale),
                 color: (diskTempC ?? 0) >= diskTempWarningC ? StatusLevel.warning.color : nil,
                 missing: diskTempC == nil)
            cell("CPU", StatusFormat.percent(cpuPercent, fractionDigits: 0, locale: locale),
                 missing: cpuPercent == nil)
            cell("RAM", StatusFormat.percent(ramPercent, fractionDigits: 0, locale: locale),
                 missing: ramPercent == nil)
            cell("USV", StatusFormat.percent(upsChargePercent, fractionDigits: 0, locale: locale),
                 color: upsOnBattery ? StatusLevel.critical.color : nil,
                 missing: upsChargePercent == nil)
            cell("Backup", StatusFormat.backupAge(lastBackup, now: now), missing: lastBackup == nil)
        }
    }

    private func cell(_ title: String, _ value: String, color: Color? = nil, missing: Bool) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title.uppercased())
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.subheadline.weight(.semibold))
                .monospacedDigit()
                .foregroundStyle(missing ? AnyShapeStyle(.secondary) : AnyShapeStyle(color ?? .primary))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

private let previewNow = Date(timeIntervalSince1970: 1_800_000_000)

#Preview("Kennzahlen hell") {
    MetricsRowPreview().preferredColorScheme(.light)
}

#Preview("Kennzahlen dunkel") {
    MetricsRowPreview()
        .preferredColorScheme(.dark)
        .background(Color.black)
}

private struct MetricsRowPreview: View {
    var body: some View {
        VStack(spacing: 20) {
            MetricsRow(
                diskTempC: 52, cpuPercent: 9, ramPercent: 41, upsChargePercent: 100,
                lastBackup: previewNow.addingTimeInterval(-9 * 3600), now: previewNow)
            MetricsRow(
                diskTempC: 38, cpuPercent: 71, ramPercent: 88, upsChargePercent: 64,
                upsOnBattery: true,
                lastBackup: previewNow.addingTimeInterval(-3 * 86400), now: previewNow)
            MetricsRow(now: previewNow)
        }
        .environment(\.locale, Locale(identifier: "de_DE"))
        .padding()
        .frame(width: 360)
    }
}
