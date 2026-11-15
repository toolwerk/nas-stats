import SwiftUI

/// Fill bar of a volume with percentage, used and total size and a forecast
/// line (#16). The tick marks the critical threshold.
struct FillBar: View {
    enum Detail {
        /// Percentage with one digit plus "19,0 von 21,8 TB".
        case full
        /// Whole percentage only, for small widgets.
        case percent
    }

    let name: String
    let percent: Double?
    var usedBytes: UInt64?
    var totalBytes: UInt64?
    var forecast: FillForecast?
    var detail: Detail = .full
    var barHeight: CGFloat = 8
    var thresholds: FillThresholds = .standard

    @Environment(\.locale) private var locale
    @Environment(\.timeZone) private var timeZone

    private var level: StatusLevel { thresholds.level(for: percent) }

    private var valueText: String {
        switch detail {
        case .percent:
            // Round down so the text never reaches a threshold the colour has not.
            return StatusFormat.percent(percent.map { $0.rounded(.down) }, fractionDigits: 0, locale: locale)
        case .full:
            let p = StatusFormat.percent(percent, fractionDigits: 1, locale: locale)
            guard let capacity = StatusFormat.capacity(used: usedBytes, total: totalBytes, locale: locale)
            else { return p }
            return "\(p) · \(capacity)"
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(name).fontWeight(.medium)
                Spacer(minLength: 8)
                Text(valueText)
                    .monospacedDigit()
                    .foregroundStyle(percent == nil ? .secondary : .primary)
            }
            .font(.footnote)
            bar
            if let forecast {
                forecastLine(forecast)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var bar: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let fraction = min(max((percent ?? 0) / 100, 0), 1)
            ZStack(alignment: .leading) {
                Capsule().fill(Color("FillTrack"))
                if percent != nil {
                    Capsule()
                        .fill(level.color)
                        .frame(width: width * fraction)
                }
                Rectangle()
                    .fill(.background)
                    .frame(width: 2)
                    .offset(x: width * thresholds.critical / 100 - 1)
            }
            .clipShape(Capsule())
        }
        .frame(height: barHeight)
    }

    private func forecastLine(_ forecast: FillForecast) -> some View {
        let text = StatusFormat.forecast(forecast, locale: locale, timeZone: timeZone)
        let tail = text.date.map { ", \($0)" } ?? ""
        return Text("\(Text(text.main).fontWeight(.semibold))\(tail)")
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

private let previewFull: UInt64 = 24_000_000_000_000

#Preview("Füllstandsbalken hell") {
    FillBarPreview().preferredColorScheme(.light)
}

#Preview("Füllstandsbalken dunkel") {
    FillBarPreview()
        .preferredColorScheme(.dark)
        .background(Color.black)
}

private struct FillBarPreview: View {
    var body: some View {
        VStack(spacing: 16) {
            FillBar(
                name: "DataVol1", percent: 87.1,
                usedBytes: 20_904_000_000_000, totalBytes: previewFull,
                forecast: .full(
                    days: 143,
                    date: Date(timeIntervalSince1970: 1_807_000_000)))
            FillBar(
                name: "tank", percent: 64, usedBytes: 15_360_000_000_000, totalBytes: previewFull,
                forecast: .noTrend)
            FillBar(name: "Archiv", percent: 81, detail: .percent)
            FillBar(
                name: "Backup", percent: 93.4,
                usedBytes: 22_416_000_000_000, totalBytes: previewFull,
                forecast: .full(days: 0.4, date: nil))
            FillBar(name: "Neu", percent: nil)
        }
        .environment(\.locale, Locale(identifier: "de_DE"))
        .padding()
        .frame(width: 340)
    }
}
