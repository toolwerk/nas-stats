import Foundation

/// How a line under the ring of the small widget is drawn.
enum WidgetLineStyle: Equatable, Sendable {
    /// Why the NAS has no current state.
    case problem
    /// The first finding, in the colour of its level.
    case finding(StatusLevel)
    /// "voll in 143 Tagen".
    case forecast
    /// "Stand 14:02" of an old state.
    case stale
}

/// One line of text of a widget.
struct WidgetLine: Equatable, Sendable {
    var text: String
    var style: WidgetLineStyle
}

/// What the widgets "NAS" show of a NAS (#24). No WidgetKit here, the tests
/// compile `Shared` but not the widget target.
extension WidgetNas {
    /// Most volumes the medium widget has room for.
    static let mediumVolumeLimit = 3

    /// The volume with the highest fill level, for the ring. Volumes
    /// without a percentage do not count; on a tie the first one wins.
    var fullestVolume: WidgetVolume? {
        var best: WidgetVolume?
        for volume in volumes {
            guard let percent = volume.usedPercent else { continue }
            if let current = best?.usedPercent, current >= percent { continue }
            best = volume
        }
        return best
    }

    /// The volumes of the medium widget: the fullest first (equal ones keep
    /// their order), those without a percentage last, at most
    /// `mediumVolumeLimit`.
    var mediumVolumes: [WidgetVolume] {
        let known = volumes.filter { $0.usedPercent != nil }
            .enumerated()
            .sorted { lhs, rhs in
                let (l, r) = (lhs.element.usedPercent ?? 0, rhs.element.usedPercent ?? 0)
                return l != r ? l > r : lhs.offset < rhs.offset
            }
            .map(\.element)
        let unknown = volumes.filter { $0.usedPercent == nil }
        return Array((known + unknown).prefix(Self.mediumVolumeLimit))
    }

    /// The error if there is one (it explains a red or dashed dot, while
    /// the findings may be those of an old state), else the first finding.
    var messageLine: WidgetLine? {
        if let problemLine { return WidgetLine(text: problemLine, style: .problem) }
        if let firstFinding {
            return WidgetLine(text: firstFinding, style: .finding(firstFindingLevel ?? .warning))
        }
        return nil
    }

    /// The lines under the ring of the small widget, at most two. With a
    /// message: the message, then "Stand" if the state is old, else the
    /// forecast. Without: the forecast, then "Stand".
    func smallLines(
        in snapshot: WidgetSnapshot, now: Date,
        locale: Locale = .current, timeZone: TimeZone = .current
    ) -> [WidgetLine] {
        var forecast: WidgetLine?
        if let value = fullestVolume?.forecast, case .full = value {
            forecast = WidgetLine(
                text: StatusFormat.forecast(value, locale: locale, timeZone: timeZone).main,
                style: .forecast)
        }
        var stale: WidgetLine?
        if let observedAt, snapshot.isStale(self, now: now) {
            stale = WidgetLine(
                text: StatusFormat.observed(observedAt, now: now, locale: locale, timeZone: timeZone),
                style: .stale)
        }
        if let messageLine { return [messageLine, stale ?? forecast].compactMap { $0 } }
        return [forecast, stale].compactMap { $0 }
    }
}
