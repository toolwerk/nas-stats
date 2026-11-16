import SwiftUI
import WidgetKit

/// Medium widget „NAS“ (#24): left status dot, name, model and the first
/// warning, right the fullest volumes as bars, below disk temperature, CPU,
/// RAM and the last backup.
struct NasMediumView: View {
    let nas: WidgetNas
    let snapshot: WidgetSnapshot
    let now: Date

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .top, spacing: 14) {
                summary
                volumes
            }
            MetricsRow(
                diskTempC: nas.hottestDiskC, cpuPercent: nas.cpuPercent,
                ramPercent: nas.memoryPercent, lastBackup: nas.lastBackupAt, now: now,
                diskTempWarningC: snapshot.thresholds.diskTempWarningC, showsUps: false)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var summary: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                StatusDot(level: nas.status)
                    .widgetAccentable()
                Text(nas.name)
                    .font(.headline)
                    .lineLimit(1)
                if snapshot.demo {
                    Text("DEMO")
                        .font(.caption2.bold())
                        .foregroundStyle(StatusLevel.warning.color)
                }
            }
            if let model = nas.model {
                Text(model)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if let line = nas.messageLine {
                Text(line.text)
                    .font(.caption)
                    .fontWeight(line.style == .problem ? .regular : .semibold)
                    .foregroundStyle(color(of: line))
                    .lineLimit(2)
            }
            Spacer(minLength: 0)
            if let observedAt = nas.observedAt, snapshot.isStale(nas, now: now) {
                Text(StatusFormat.observed(observedAt, now: now))
                    .font(.caption)
                    .foregroundStyle(StatusLevel.offline.color)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private var volumes: some View {
        let shown = nas.mediumVolumes
        VStack(alignment: .leading, spacing: 5) {
            if shown.isEmpty {
                Text("Keine Volumes")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(shown) { volume in
                    FillBar(
                        name: volume.name, percent: volume.usedPercent, detail: .percent,
                        barHeight: 5, thresholds: snapshot.thresholds.fill)
                }
            }
        }
        .widgetAccentable()
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private func color(of line: WidgetLine) -> Color {
        switch line.style {
        case let .finding(level): level.color
        default: Color.secondary
        }
    }
}
