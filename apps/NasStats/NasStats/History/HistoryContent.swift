import Foundation

extension HistoryRange {
    /// The ranges of the history screen, shortest first. The core keeps 30
    /// days of hourly values and a year of daily ones.
    static let options: [HistoryRange] = [.day, .week, .month, .year]

    /// Label of the picker.
    var label: String {
        switch self {
        case .day: "24 h"
        case .week: "7 Tage"
        case .month: "30 Tage"
        case .year: "1 Jahr"
        }
    }

    /// Length in days; also the key the screen saves its choice under.
    var days: Int {
        switch self {
        case .day: 1
        case .week: 7
        case .month: 30
        case .year: 365
        }
    }

    /// Length in seconds.
    var span: Int64 { Int64(days) * 86_400 }

    /// The range with this length, `nil` for an unknown one.
    init?(days: Int) {
        guard let range = Self.options.first(where: { $0.days == days }) else { return nil }
        self = range
    }
}

/// What the history screen draws of the archive of the core, without
/// SwiftUI, so tests cover it (#19).
struct HistoryContent {
    /// One point of a line: a sample, an hour or a day with its spread.
    struct Point: Equatable {
        let at: Date
        let min: Double
        let mean: Double
        let max: Double
    }

    /// One line of a chart: the mean with the band from min to max.
    struct Line: Identifiable, Equatable {
        let id: String
        let name: String
        let points: [Point]
    }

    /// Fill level of one volume in percent, with the forecast.
    struct VolumeChart: Identifiable, Equatable {
        /// A point of the dashed forecast line, in percent.
        struct ForecastPoint: Equatable {
            let at: Date
            let percent: Double
        }

        /// Id of the volume.
        let id: String
        let name: String
        let line: Line
        /// Start (now) and end of the dashed line; empty without a forecast.
        let forecast: [ForecastPoint]
        /// Growth in percent of the volume per day.
        let growthPercentPerDay: Double?
        /// "voll in 143 Tagen" and "etwa 6. April 2027".
        let forecastText: (main: String, date: String?)?
        /// Days with values in the last 30, set only below 7: too few for
        /// a forecast.
        let historyDays: Int?

        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.id == rhs.id && lhs.name == rhs.name && lhs.line == rhs.line
                && lhs.forecast == rhs.forecast && lhs.growthPercentPerDay == rhs.growthPercentPerDay
                && lhs.forecastText?.main == rhs.forecastText?.main
                && lhs.forecastText?.date == rhs.forecastText?.date
                && lhs.historyDays == rhs.historyDays
        }
    }

    /// Traffic of one interface in MB/s: "ein" and "aus".
    struct NetworkChart: Identifiable, Equatable {
        let id: String
        let lines: [Line]
    }

    /// Days of values needed before a forecast is shown.
    static let minimumForecastDays = 7

    let volumes: [VolumeChart]
    /// Disks in the order of the snapshot, then the system.
    let temperatures: [Line]
    /// "CPU" and "RAM" in percent.
    let load: [Line]
    let networks: [NetworkChart]

    /// No line has a point in the range.
    var isEmpty: Bool {
        volumes.isEmpty && temperatures.isEmpty && load.isEmpty && networks.isEmpty
    }

    /// - Parameters:
    ///   - snapshot: Last state of the NAS, for names and volumes.
    ///   - range: Range of the charts, ending `now`.
    ///   - now: Time of the screen.
    ///   - load: History of a metric in a range; errors of the core are
    ///     thrown.
    ///   - forecast: Forecast of a volume id; an error counts as none.
    init(
        snapshot: Snapshot,
        range: HistoryRange,
        now: Date,
        load: (Metric, HistoryRange) throws -> [HistorySeries],
        forecast: (String) throws -> Forecast?
    ) throws {
        let nowSeconds = Int64(now.timeIntervalSince1970)
        let used = try load(.volumeUsed, range)
        let totals = try load(.volumeTotal, range)
        // Daily values of the year: what a forecast stands on.
        let daily = range == .year ? used : try load(.volumeUsed, .year)

        volumes = NasEntry.capacityVolumes(of: snapshot).compactMap { volume in
            Self.volumeChart(
                volume, used: used, totals: totals, daily: daily, range: range,
                now: nowSeconds, forecast: forecast)
        }

        var temperatures = Self.temperatureLines(
            try load(.diskTemp, range), disks: snapshot.disks)
        temperatures += try load(.sysTemp, range).compactMap { series in
            Self.line(id: "sys", name: "System", points: series.points, scale: 1)
        }
        self.temperatures = temperatures

        var load_: [Line] = []
        for (metric, name) in [(Metric.cpu, "CPU"), (Metric.mem, "RAM")] {
            for series in try load(metric, range) {
                if let line = Self.line(id: name, name: name, points: series.points, scale: 1) {
                    load_.append(line)
                }
            }
        }
        self.load = load_

        networks = Self.networkCharts(
            rx: try load(.netRx, range), tx: try load(.netTx, range))
    }

    // MARK: - Building

    private static func line(id: String, name: String, points: [HistoryPoint], scale: Double) -> Line? {
        guard !points.isEmpty else { return nil }
        return Line(
            id: id, name: name,
            points: points.map {
                Point(
                    at: Date(timeIntervalSince1970: TimeInterval($0.at)),
                    min: $0.min * scale, mean: $0.mean * scale, max: $0.max * scale)
            })
    }

    private static func volumeChart(
        _ volume: Volume,
        used: [HistorySeries],
        totals: [HistorySeries],
        daily: [HistorySeries],
        range: HistoryRange,
        now: Int64,
        forecast: (String) throws -> Forecast?
    ) -> VolumeChart? {
        let points = used.first { $0.key == volume.id }?.points ?? []
        let totalPoints = totals.first { $0.key == volume.id }?.points ?? []
        let totalNow = volume.totalBytes.map { Double($0) } ?? totalPoints.last?.mean

        let percentPoints: [HistoryPoint] = points.compactMap { point in
            // The size at the same time; the current one if it was not kept.
            let size = totalPoints.first { $0.at == point.at }?.mean ?? totalNow
            guard let size, size > 0 else { return nil }
            return HistoryPoint(
                at: point.at, min: point.min * 100 / size, mean: point.mean * 100 / size,
                max: point.max * 100 / size, count: point.count)
        }
        guard let line = line(id: volume.id, name: volume.name, points: percentPoints, scale: 1)
        else { return nil }

        // Days with values among the last 30, today counted.
        let todayStart = now - now % 86_400
        let since = todayStart - 29 * 86_400
        let days = (daily.first { $0.key == volume.id }?.points ?? []).filter { $0.at >= since }.count
        guard days >= minimumForecastDays else {
            return VolumeChart(
                id: volume.id, name: volume.name, line: line, forecast: [],
                growthPercentPerDay: nil, forecastText: nil, historyDays: days)
        }
        guard let totalNow, totalNow > 0, let trend = try? forecast(volume.id) else {
            return VolumeChart(
                id: volume.id, name: volume.name, line: line, forecast: [],
                growthPercentPerDay: nil, forecastText: nil, historyDays: nil)
        }

        let growthPercent = trend.growthPerDay / totalNow * 100
        let startPercent = Swift.min(
            100, Swift.max(0, (totalNow - trend.growthPerDay * trend.daysToFull) / totalNow * 100))
        let horizon = now + range.span
        let end: VolumeChart.ForecastPoint
        if trend.fullAt <= horizon {
            end = .init(at: Date(timeIntervalSince1970: TimeInterval(trend.fullAt)), percent: 100)
        } else {
            // The line stops at the edge of the chart instead of years ahead.
            let ahead = Double(range.span) / 86_400
            end = .init(
                at: Date(timeIntervalSince1970: TimeInterval(horizon)),
                percent: Swift.min(100, startPercent + growthPercent * ahead))
        }
        return VolumeChart(
            id: volume.id, name: volume.name, line: line,
            forecast: [
                .init(at: Date(timeIntervalSince1970: TimeInterval(now)), percent: startPercent),
                end,
            ],
            growthPercentPerDay: growthPercent,
            forecastText: StatusFormat.forecast(
                .full(
                    days: trend.daysToFull,
                    date: Date(timeIntervalSince1970: TimeInterval(trend.fullAt)))),
            historyDays: nil)
    }

    /// Disks by the order and names of the snapshot; a key it does not
    /// know keeps the key as name, at the end.
    private static func temperatureLines(_ series: [HistorySeries], disks: [Disk]) -> [Line] {
        let byKey = Dictionary(
            series.compactMap { s in s.key.map { ($0, s.points) } },
            uniquingKeysWith: { first, _ in first })
        var lines: [Line] = []
        for disk in disks {
            if let points = byKey[disk.id], let line = line(id: disk.id, name: disk.name, points: points, scale: 1) {
                lines.append(line)
            }
        }
        let known = Set(disks.map(\.id))
        for key in byKey.keys.sorted() where !known.contains(key) {
            if let line = line(id: key, name: key, points: byKey[key] ?? [], scale: 1) {
                lines.append(line)
            }
        }
        return lines
    }

    /// One chart per interface with traffic; the sum over all interfaces
    /// is not formed, because min and max do not add up.
    private static func networkCharts(rx: [HistorySeries], tx: [HistorySeries]) -> [NetworkChart] {
        let megabytes = 1.0 / 1_000_000
        let keys = Set((rx + tx).compactMap(\.key)).sorted()
        return keys.compactMap { key in
            let directions = [("ein", rx), ("aus", tx)].compactMap { name, series -> (String, [HistoryPoint])? in
                guard let points = series.first(where: { $0.key == key })?.points, !points.isEmpty
                else { return nil }
                return (name, points)
            }
            // An interface without traffic in both directions has no chart.
            guard directions.contains(where: { $0.1.contains { $0.max > 0 } }) else { return nil }
            let lines = directions.compactMap { name, points in
                line(id: "\(key)-\(name)", name: name, points: points, scale: megabytes)
            }
            return lines.isEmpty ? nil : NetworkChart(id: key, lines: lines)
        }
    }
}
