import SwiftUI
import WidgetKit

/// Small widget „NAS“ (#24): status dot and name, the ring of the fullest
/// volume, below it the first warning or the forecast and, once the state is
/// old, its time.
struct NasSmallView: View {
    let nas: WidgetNas
    let snapshot: WidgetSnapshot
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                StatusDot(level: nas.status)
                    .widgetAccentable()
                Text(nas.name)
                    .font(.headline)
                    .lineLimit(1)
                Spacer(minLength: 0)
                if snapshot.demo {
                    Text("DEMO")
                        .font(.caption2.bold())
                        .foregroundStyle(StatusLevel.warning.color)
                }
            }
            FillRing(
                percent: nas.fullestVolume?.usedPercent,
                offline: nas.status == .offline,
                thresholds: snapshot.thresholds.fill
            )
            .widgetAccentable()
            .frame(maxWidth: .infinity)
            VStack(alignment: .leading, spacing: 1) {
                ForEach(Array(nas.smallLines(in: snapshot, now: now).enumerated()), id: \.offset) { _, line in
                    lineView(line)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private func lineView(_ line: WidgetLine) -> some View {
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
}
