import SwiftUI

/// Dot in the colour of a state. "Nicht erreichbar" is a dashed outline.
struct StatusDot: View {
    let level: StatusLevel
    var size: CGFloat = 9

    var body: some View {
        Group {
            if level == .offline {
                Circle()
                    .inset(by: 1)
                    .stroke(level.color, style: StrokeStyle(lineWidth: 2, dash: [2, 2]))
            } else {
                Circle().fill(level.color)
            }
        }
        .frame(width: size, height: size)
        .accessibilityElement()
        .accessibilityLabel("Zustand: \(level.label)")
    }
}

#Preview("Ampelpunkt hell") {
    StatusDotPreview()
        .preferredColorScheme(.light)
}

#Preview("Ampelpunkt dunkel") {
    StatusDotPreview()
        .preferredColorScheme(.dark)
        .background(Color.black)
}

private struct StatusDotPreview: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(StatusLevel.allCases, id: \.self) { level in
                HStack(spacing: 8) {
                    StatusDot(level: level, size: 12)
                    Text(level.label)
                }
            }
        }
        .padding()
    }
}
