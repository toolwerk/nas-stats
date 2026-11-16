#if os(iOS)
import SwiftUI
import WidgetKit

/// Round lock screen widget „NAS“ (#25): the ring of the fullest volume. The
/// lock screen shows only white in steps, so the ring is white, not coloured;
/// the state is in words in the other two shapes.
struct NasCircularView: View {
    let nas: WidgetNas?
    let demo: Bool

    var body: some View {
        ZStack {
            AccessoryWidgetBackground()
            FillRing(
                percent: nas?.fullestVolume?.usedPercent,
                lineFraction: 0.12,
                tint: .white,
                offline: nas?.status == .offline,
                showsPercentSign: false,
                trackColor: .white.opacity(0.3)
            )
            .widgetAccentable()
            .padding(4)
            if demo {
                Text("DEMO")
                    .font(.system(size: 7, weight: .bold))
                    .frame(maxHeight: .infinity, alignment: .bottom)
                    .padding(.bottom, 3)
            }
        }
    }
}

/// Rectangular lock screen widget: name and state, the first message, the
/// fullest volume.
struct NasRectangularView: View {
    let nas: WidgetNas
    let snapshot: WidgetSnapshot
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 4) {
                Text(nas.lockTitle)
                    .font(.headline)
                    .lineLimit(1)
                    .widgetAccentable()
                if snapshot.demo {
                    Spacer(minLength: 0)
                    Text("DEMO").font(.caption2.bold())
                }
            }
            ForEach(Array(nas.lockLines(in: snapshot, now: now).enumerated()), id: \.offset) { _, line in
                Text(line).font(.caption).lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }
}

/// Line above the clock: "Keller · 87 % · Warnung". iOS draws no background
/// and cuts the text when it does not fit.
struct NasInlineView: View {
    let nas: WidgetNas
    let demo: Bool

    var body: some View {
        Text(nas.lockInline(demo: demo))
    }
}
#endif
