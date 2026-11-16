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

    /// Fill level of the fullest volume for the lock screen widgets (#25),
    /// rounded down. `nil` when the NAS is not reachable (its volumes are
    /// those of an old state) or has no volume with a percentage.
    var lockPercent: Int? {
        guard status != .offline, let percent = fullestVolume?.usedPercent else { return nil }
        return Int(StatusFormat.floored(percent, fractionDigits: 0) ?? 0)
    }

    /// The inline lock screen widget: "Keller · 87 % · Warnung", with "DEMO"
    /// in front for demo values and without the percentage when there is none.
    func lockInline(demo: Bool) -> String {
        var parts: [String] = []
        if demo { parts.append("DEMO") }
        parts.append(name)
        if let lockPercent { parts.append("\(lockPercent) %") }
        parts.append(status.label)
        return parts.joined(separator: " · ")
    }

    /// Headline of the rectangular lock screen widget: "Keller · Warnung".
    var lockTitle: String { "\(name) · \(status.label)" }

    /// The lines under the headline of the rectangular lock screen widget, at
    /// most two: the first message (unless it only repeats the state), then
    /// the fullest volume with its percentage and the forecast, or "Stand"
    /// once the state is old. Without a percentage only "Stand" is left.
    func lockLines(
        in snapshot: WidgetSnapshot, now: Date,
        locale: Locale = .current, timeZone: TimeZone = .current
    ) -> [String] {
        var lines: [String] = []
        if let text = messageLine?.text, text != status.label { lines.append(text) }
        var stale: String?
        if let observedAt, snapshot.isStale(self, now: now) {
            stale = StatusFormat.observed(observedAt, now: now, locale: locale, timeZone: timeZone)
        }
        if let volume = fullestVolume, let lockPercent {
            var fill = "\(volume.name) \(lockPercent) %"
            if let stale {
                fill += " · \(stale)"
            } else if let value = volume.forecast, case .full = value {
                fill += " · " + StatusFormat.forecast(value, locale: locale, timeZone: timeZone).main
            }
            lines.append(fill)
        } else if let stale {
            lines.append(stale)
        }
        return lines
    }
}
