import SwiftUI
import WidgetKit

/// Small widget „Füllstand“ (#38): status dot and name, a bar per volume of
/// the chosen NAS (the three fullest) and below the sum of their sizes; once
/// the state is old, its time replaces the sum.
struct FillSmallView: View {
    let nas: WidgetNas
    let snapshot: WidgetSnapshot
    let now: Date

    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
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
            volumes
            Spacer(minLength: 0)
            footer
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
                        barHeight: 10, thresholds: snapshot.thresholds.fill)
                }
            }
        }
        .widgetAccentable()
    }

    @ViewBuilder private var footer: some View {
        if let observedAt = nas.observedAt, snapshot.isStale(nas, now: now) {
            Text(StatusFormat.observed(observedAt, now: now, locale: locale))
                .font(.caption)
                .foregroundStyle(StatusLevel.offline.color)
        } else if let line = nas.capacityLine(locale: locale) {
            Text(line)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}
