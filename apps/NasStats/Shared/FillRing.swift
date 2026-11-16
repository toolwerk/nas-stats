import SwiftUI

/// Ring for the highest fill level of a NAS (small widget, round lock screen
/// widget). Starts at 12 o'clock, runs clockwise (#16).
struct FillRing: View {
    let percent: Double?
    var lineFraction: CGFloat = 0.11
    /// Replaces the threshold colour, e.g. for a tinted lock screen.
    var tint: Color?
    /// NAS not reachable: dashed grey track, no arc.
    var offline = false
    var showsPercentSign = true
    var thresholds: FillThresholds = .standard
    /// Replaces the track colour, e.g. white for the lock screen.
    var trackColor: Color?

    private var color: Color { tint ?? thresholds.level(for: percent).color }

    var body: some View {
        GeometryReader { proxy in
            let side = min(proxy.size.width, proxy.size.height)
            let line = side * lineFraction
            ZStack {
                if offline {
                    Circle()
                        .inset(by: line / 2)
                        .stroke(
                            tint ?? StatusLevel.offline.color,
                            style: StrokeStyle(lineWidth: line, dash: [line * 0.7, line * 0.7]))
                } else {
                    Circle()
                        .inset(by: line / 2)
                        .stroke(trackColor ?? Color("FillTrack"), lineWidth: line)
                    if let percent {
                        Circle()
                            .inset(by: line / 2)
                            .trim(from: 0, to: min(max(percent / 100, 0), 1))
                            .stroke(color, style: StrokeStyle(lineWidth: line, lineCap: .round))
                            .rotationEffect(.degrees(-90))
                    }
                }
                Text(centerText)
                    .font(.system(size: side * 0.23, weight: .bold))
                    .monospacedDigit()
                    .minimumScaleFactor(0.5)
                    .foregroundStyle(offline || percent == nil ? .secondary : .primary)
            }
            .frame(width: side, height: side)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityElement()
        .accessibilityLabel(offline ? "Nicht erreichbar" : "Füllstand \(centerText)")
    }

    private var centerText: String {
        if offline { return StatusFormat.notAvailable }
        guard let percent else { return StatusFormat.notAvailable }
        // Round down: rounding up would show "90 %" in yellow.
        let whole = Int(StatusFormat.floored(percent, fractionDigits: 0) ?? 0)
        return showsPercentSign ? "\(whole) %" : "\(whole)"
    }
}

#Preview("Ring hell") {
    FillRingPreview().preferredColorScheme(.light)
}

#Preview("Ring dunkel") {
    FillRingPreview()
        .preferredColorScheme(.dark)
        .background(Color.black)
}

private struct FillRingPreview: View {
    var body: some View {
        HStack(spacing: 16) {
            FillRing(percent: 64)
            FillRing(percent: 87)
            FillRing(percent: 93)
            FillRing(percent: nil)
            FillRing(percent: nil, offline: true)
        }
        .frame(height: 80)
        .padding()
    }
}
