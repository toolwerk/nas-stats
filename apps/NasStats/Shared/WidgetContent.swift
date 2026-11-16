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

/// What the widgets "Füllstand" and "Alle NAS" show of a NAS (#38).
extension WidgetNas {
    /// "14,0 von 22,5 TB": the sum of the volumes that have used and total
    /// size; `nil` if none has.
    func capacityLine(locale: Locale = .current) -> String? {
        let sized = volumes.compactMap { volume -> (UInt64, UInt64)? in
            guard let used = volume.usedBytes, let total = volume.totalBytes else { return nil }
            return (used, total)
        }
        guard !sized.isEmpty else { return nil }
        return StatusFormat.capacity(
            used: sized.reduce(0) { $0 + $1.0 }, total: sized.reduce(0) { $0 + $1.1 },
            locale: locale)
    }

    /// The text at the right of a row of the large widget; the first rule
    /// that applies wins: "Stand 14:02" of an old state, the error, the
    /// number of warnings in the colour of the first one, else nothing.
    func rowStatus(
        in snapshot: WidgetSnapshot, now: Date,
        locale: Locale = .current, timeZone: TimeZone = .current
    ) -> WidgetLine? {
        if let observedAt, snapshot.isStale(self, now: now) {
            return WidgetLine(
                text: StatusFormat.observed(observedAt, now: now, locale: locale, timeZone: timeZone),
                style: .stale)
        }
        if let problemLine { return WidgetLine(text: problemLine, style: .problem) }
        if findingCount > 0 {
            return WidgetLine(
                text: findingCount == 1 ? "1 Warnung" : "\(findingCount) Warnungen",
                style: .finding(firstFindingLevel ?? .warning))
        }
        return nil
    }

    /// The line under the bar of a row: "DataVol1 87 % · Backup vor 9 h".
    /// Without a volume with a percentage only the backup part.
    func rowDetail(now: Date, locale: Locale = .current) -> String {
        let backup = "Backup \(StatusFormat.backupAge(lastBackupAt, now: now))"
        guard let volume = fullestVolume else { return backup }
        let percent = StatusFormat.percent(
            StatusFormat.floored(volume.usedPercent, fractionDigits: 0), fractionDigits: 0,
            locale: locale)
        return "\(volume.name) \(percent) · \(backup)"
    }
}

extension WidgetSnapshot {
    /// Most NAS the large widget has room for next to the history.
    static let largeNasLimit = 3

    /// The rows of the large widget: the worst NAS first (the snapshot is
    /// sorted that way), at most `largeNasLimit`.
    var largeRows: [WidgetNas] { Array(nas.prefix(Self.largeNasLimit)) }

    /// How many NAS the large widget leaves out.
    var largeHiddenCount: Int { max(0, nas.count - Self.largeNasLimit) }

    /// The NAS and its volume the history below the rows belongs to: the
    /// fullest volume of all NAS, on a tie the first one. `nil` if no volume
    /// has a percentage.
    var historySource: (nas: WidgetNas, volume: WidgetVolume)? {
        var best: (nas: WidgetNas, volume: WidgetVolume)?
        for candidate in nas {
            guard let volume = candidate.fullestVolume, let percent = volume.usedPercent else {
                continue
            }
            if let current = best?.volume.usedPercent, current >= percent { continue }
            best = (candidate, volume)
        }
        return best
    }

    /// The history needs two points for a line.
    static let minimumHistoryPoints = 2
}
