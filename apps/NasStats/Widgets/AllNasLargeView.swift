import Charts
import SwiftUI
import WidgetKit

/// Large widget „Alle NAS“ (#38): a row per NAS (the worst first, at most
/// three) with status dot, name, the bar of its fullest volume, the number of
/// warnings and the age of the last backup, below the 30 days of the fullest
/// volume of all NAS.
struct AllNasLargeView: View {
    let snapshot: WidgetSnapshot
    let now: Date

    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Alle NAS")
                    .font(.headline)
                Spacer(minLength: 0)
                if snapshot.demo {
                    Text("DEMO")
                        .font(.caption2.bold())
                        .foregroundStyle(StatusLevel.warning.color)
                }
            }
            ForEach(snapshot.largeRows) { nas in
                row(nas)
            }
            if snapshot.largeHiddenCount > 0 {
                Text(snapshot.largeHiddenCount == 1
                    ? "+ 1 weiteres NAS" : "+ \(snapshot.largeHiddenCount) weitere NAS")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            history
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func row(_ nas: WidgetNas) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                StatusDot(level: nas.status)
                    .widgetAccentable()
                Text(nas.name)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Spacer(minLength: 8)
                if let status = nas.rowStatus(in: snapshot, now: now, locale: locale) {
                    statusView(status)
                }
            }
            RowBar(
                percent: nas.fullestVolume?.usedPercent, offline: nas.status == .offline,
                thresholds: snapshot.thresholds.fill)
                .widgetAccentable()
            Text(nas.rowDetail(now: now, locale: locale))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    @ViewBuilder private func statusView(_ line: WidgetLine) -> some View {
        let text = Text(line.text).font(.caption).lineLimit(1)
        switch line.style {
        case .problem, .forecast:
            text.foregroundStyle(.secondary)
        case let .finding(level):
            text.fontWeight(.semibold).foregroundStyle(level.color)
        case .stale:
            text.foregroundStyle(StatusLevel.offline.color)
        }
    }

    @ViewBuilder private var history: some View {
        if let source = snapshot.historySource {
            VStack(alignment: .leading, spacing: 3) {
                Text("\(source.nas.name) · \(source.volume.name) · 30 Tage")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if source.nas.fullestVolumeHistory.count >= WidgetSnapshot.minimumHistoryPoints {
                    HistoryChart(
                        points: source.nas.fullestVolumeHistory,
                        color: snapshot.thresholds.fill.level(for: source.volume.usedPercent).color,
                        locale: locale)
                } else {
                    Text("Noch kein Verlauf")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 84)
                }
            }
        }
    }
}

/// Thin bar of the fill level of a row; the track alone without a value.
private struct RowBar: View {
    let percent: Double?
    let offline: Bool
    let thresholds: FillThresholds

    var body: some View {
        GeometryReader { proxy in
            let fraction = min(max((percent ?? 0) / 100, 0), 1)
            ZStack(alignment: .leading) {
                Capsule().fill(Color("FillTrack"))
                if percent != nil {
                    Capsule()
                        .fill(offline ? StatusLevel.offline.color : thresholds.level(for: percent).color)
                        .frame(width: proxy.size.width * fraction)
                }
            }
        }
        .frame(height: 7)
    }
}

/// The fill level of the last 30 days as an area with a line.
private struct HistoryChart: View {
    let points: [WidgetFillPoint]
    let color: Color
    let locale: Locale

    private var range: ClosedRange<Double> {
        let values = points.map(\.percent)
        let low = max(0, (values.min() ?? 0).rounded(.down) - 1)
        let high = min(100, (values.max() ?? 100).rounded(.up) + 1)
        return low ... max(high, low + 1)
    }

    var body: some View {
        Chart(points, id: \.day) { point in
            AreaMark(
                x: .value("Tag", point.day), yStart: .value("Basis", range.lowerBound),
                yEnd: .value("Füllstand", point.percent))
                .foregroundStyle(color.opacity(0.2))
            LineMark(x: .value("Tag", point.day), y: .value("Füllstand", point.percent))
                .foregroundStyle(color)
        }
        .chartYScale(domain: range)
        .chartXAxis {
            AxisMarks(values: xMarks) { value in
                AxisValueLabel(anchor: value.index == 0 ? .topLeading : .topTrailing) {
                    if let day = value.as(Date.self) {
                        Text(value.index == 0 ? day.formatted(.dateTime.day().month(.abbreviated).locale(locale)) : "heute")
                            .font(.caption2)
                    }
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading, values: .automatic(desiredCount: 2)) { _ in
                AxisGridLine()
                AxisValueLabel().font(.caption2)
            }
        }
        .frame(height: 84)
    }

    private var xMarks: [Date] {
        guard let first = points.first?.day, let last = points.last?.day else { return [] }
        return [first, last]
    }
}
