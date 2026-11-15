import Foundation

/// Texts of the status building blocks. Pure functions, so tests cover them
/// without rendering a view (#16).
enum StatusFormat {
    static let notAvailable = "n/a"

    /// "87,1 %"; a missing value is "n/a".
    static func percent(_ value: Double?, fractionDigits: Int = 1, locale: Locale = .current) -> String {
        guard let value else { return notAvailable }
        let number = value.formatted(
            .number.precision(.fractionLength(fractionDigits)).locale(locale))
        return "\(number) %"
    }

    /// Rounds down to `fractionDigits`, so a shown percentage never reaches a
    /// threshold that the colour (from the raw value) has not.
    static func floored(_ value: Double?, fractionDigits: Int) -> Double? {
        guard let value else { return nil }
        let factor = pow(10, Double(fractionDigits))
        return (value * factor).rounded(.down) / factor
    }

    /// "19,0 von 21,8 TB" in steps of 1024, like the NAS web interfaces.
    /// Nil if used or total is missing.
    static func capacity(used: UInt64?, total: UInt64?, locale: Locale = .current) -> String? {
        guard let used, let total else { return nil }
        let units = ["B", "KB", "MB", "GB", "TB", "PB"]
        var index = 0
        var scaled = Double(total)
        while scaled >= 1024, index < units.count - 1 {
            scaled /= 1024
            index += 1
        }
        let divisor = pow(1024, Double(index))
        let format = FloatingPointFormatStyle<Double>.number.precision(.fractionLength(1)).locale(locale)
        return "\((Double(used) / divisor).formatted(format)) von \(scaled.formatted(format)) \(units[index])"
    }

    /// "52 °C"; a missing value is "n/a".
    static func temperature(_ celsius: Double?, locale: Locale = .current) -> String {
        guard let celsius else { return notAvailable }
        return "\(celsius.formatted(.number.precision(.fractionLength(0)).locale(locale))) °C"
    }

    /// Main line of the forecast ("voll in 143 Tagen") and the date part
    /// ("etwa 6. April 2027") if there is one.
    static func forecast(
        _ forecast: FillForecast,
        locale: Locale = .current,
        timeZone: TimeZone = .current
    ) -> (main: String, date: String?) {
        switch forecast {
        case .noTrend:
            return ("kein Trend", nil)
        case let .full(days, date):
            let main: String
            if days < 1 {
                main = "heute voll"
            } else if days < 2 {
                main = "voll in 1 Tag"
            } else if days < 365 {
                main = "voll in \(Int(days)) Tagen"
            } else {
                let years = (days / 365).formatted(
                    .number.precision(.fractionLength(1)).locale(locale))
                main = "voll in \(years) Jahren"
            }
            guard let date else { return (main, nil) }
            var style = Date.FormatStyle(date: .long, time: .omitted, locale: locale, timeZone: timeZone)
            style.calendar = Calendar(identifier: .gregorian)
            return (main, "etwa \(date.formatted(style))")
        }
    }

    /// Uptime: "45 min" below an hour, "30 h" below two days, else
    /// "38 Tage"; a missing value is "n/a" (#18).
    static func uptime(_ seconds: UInt64?) -> String {
        guard let seconds else { return notAvailable }
        if seconds < 3600 { return "\(seconds / 60) min" }
        if seconds < 48 * 3600 { return "\(seconds / 3600) h" }
        return "\(seconds / 86400) Tage"
    }

    /// Power-on hours of a disk: "21.412 h"; a missing value is "n/a".
    static func hours(_ hours: UInt64?, locale: Locale = .current) -> String {
        guard let hours else { return notAvailable }
        return "\(hours.formatted(.number.grouping(.automatic).locale(locale))) h"
    }

    /// Remaining runtime of a UPS: "34 min" or "1 h 05 min"; a missing
    /// value is "n/a".
    static func runtime(_ seconds: UInt64?) -> String {
        guard let seconds else { return notAvailable }
        let minutes = seconds / 60
        if minutes < 60 { return "\(minutes) min" }
        let rest = minutes % 60
        return "\(minutes / 60) h \(rest < 10 ? "0" : "")\(rest) min"
    }

    /// Size in steps of 1024 with at most one decimal: "7,5 GB", "8 GB";
    /// a missing value is "n/a".
    static func bytes(_ bytes: UInt64?, locale: Locale = .current) -> String {
        guard let bytes else { return notAvailable }
        let units = ["B", "KB", "MB", "GB", "TB", "PB"]
        var index = 0
        var scaled = Double(bytes)
        // 1023,96 MB rounds to "1 GB", not "1.024 MB".
        while (scaled * 10).rounded() / 10 >= 1024, index < units.count - 1 {
            scaled /= 1024
            index += 1
        }
        let number = scaled.formatted(.number.precision(.fractionLength(0...1)).locale(locale))
        return "\(number) \(units[index])"
    }

    /// Throughput of an interface in MB/s (steps of 10^6, like the NAS web
    /// interfaces): "↓ 4,2 · ↑ 0,3 MB/s". A missing side is "n/a", both
    /// missing are "n/a" alone.
    static func throughput(rx: Double?, tx: Double?, locale: Locale = .current) -> String {
        guard rx != nil || tx != nil else { return notAvailable }
        func side(_ value: Double?) -> String {
            guard let value else { return notAvailable }
            return (value / 1_000_000).formatted(.number.precision(.fractionLength(1)).locale(locale))
        }
        return "↓ \(side(rx)) · ↑ \(side(tx)) MB/s"
    }

    /// Next scheduled run: the time alone ("02:00") on the same day,
    /// "morgen, 02:00" on the next, else with the day ("6. Apr., 02:00");
    /// a missing value is "n/a".
    static func nextRun(
        _ date: Date?,
        now: Date = .now,
        locale: Locale = .current,
        timeZone: TimeZone = .current
    ) -> String {
        guard let date else { return notAvailable }
        return dayAndTime(date, now: now, locale: locale, timeZone: timeZone, tomorrow: true)
    }

    /// Time of a snapshot: "Stand 14:05" today, else "Stand 14. Nov., 14:05",
    /// so an old snapshot kept after errors does not look current.
    static func observed(
        _ date: Date,
        now: Date = .now,
        locale: Locale = .current,
        timeZone: TimeZone = .current
    ) -> String {
        "Stand \(dayAndTime(date, now: now, locale: locale, timeZone: timeZone, tomorrow: false))"
    }

    private static func dayAndTime(
        _ date: Date, now: Date, locale: Locale, timeZone: TimeZone, tomorrow: Bool
    ) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.timeZone = timeZone
        formatter.calendar = calendar
        formatter.setLocalizedDateFormatFromTemplate("HHmm")
        let time = formatter.string(from: date)
        if calendar.isDate(date, inSameDayAs: now) { return time }
        if tomorrow, let next = calendar.date(byAdding: .day, value: 1, to: now),
           calendar.isDate(date, inSameDayAs: next) {
            return "morgen, \(time)"
        }
        formatter.setLocalizedDateFormatFromTemplate("dMMMHHmm")
        return formatter.string(from: date)
    }

    /// "vor 9 h"; a missing backup is "n/a".
    static func backupAge(_ last: Date?, now: Date = .now) -> String {
        guard let last else { return notAvailable }
        let seconds = now.timeIntervalSince(last)
        if seconds < 60 { return "gerade eben" }
        if seconds < 3600 { return "vor \(Int(seconds / 60)) min" }
        if seconds < 48 * 3600 { return "vor \(Int(seconds / 3600)) h" }
        return "vor \(Int(seconds / 86400)) Tagen"
    }
}
