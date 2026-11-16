import Foundation
import Testing

/// Charts of the history screen from the archive of the core (#19).
struct HistoryContentTests {
    /// 14 November 2026, 14:05 UTC.
    private static let nowSeconds: Int64 = 1_794_665_100
    private let now = Date(timeIntervalSince1970: TimeInterval(nowSeconds))
    private let day: Int64 = 86_400
    private let gigabyte = 1_000_000_000.0

    private func keller() -> Snapshot {
        demoSnapshots(now: Self.nowSeconds, thresholds: standardThresholds()).first { $0.nas.id == "demo-keller" }!
    }

    private func point(_ at: Int64, _ value: Double) -> HistoryPoint {
        HistoryPoint(at: at, min: value, mean: value, max: value, count: 1)
    }

    /// Series of a volume: `values` per day, oldest first, ending today.
    private func daily(_ key: String, _ values: [Double]) -> HistorySeries {
        let today = Self.nowSeconds - Self.nowSeconds % day
        let points = values.enumerated().map { index, value in
            point(today - Int64(values.count - 1 - index) * day, value)
        }
        return HistorySeries(key: key, points: points)
    }

    /// Loader with a series per metric; the same for every range.
    private func loader(_ series: [Metric: [HistorySeries]]) -> (Metric, HistoryRange) throws -> [HistorySeries] {
        { metric, _ in series[metric] ?? [] }
    }

    private func volumeId(_ snapshot: Snapshot) -> String {
        NasEntry.capacityVolumes(of: snapshot)[0].id
    }

    private func content(
        _ snapshot: Snapshot, range: HistoryRange = .month,
        series: [Metric: [HistorySeries]], forecast: Forecast? = nil
    ) throws -> HistoryContent {
        try HistoryContent(
            snapshot: snapshot, range: range, now: now, load: loader(series),
            forecast: { _ in forecast })
    }

    @Test func percentUsesTheSizeAtTheSameTime() throws {
        let snapshot = keller()
        let id = volumeId(snapshot)
        let used = daily(id, [50 * gigabyte, 60 * gigabyte])
        // The volume grew from 100 to 200 GB between the two days.
        let total = daily(id, [100 * gigabyte, 200 * gigabyte])
        let chart = try #require(
            try content(snapshot, series: [.volumeUsed: [used], .volumeTotal: [total]]).volumes.first)
        #expect(chart.line.points.map(\.mean) == [50, 30])
    }

    @Test func percentFallsBackToTheCurrentSize() throws {
        var snapshot = keller()
        let id = volumeId(snapshot)
        let index = try #require(snapshot.volumes.firstIndex { $0.id == id })
        snapshot.volumes[index].totalBytes = 200_000_000_000
        let chart = try #require(
            try content(snapshot, series: [.volumeUsed: [daily(id, [100 * gigabyte])]]).volumes.first)
        #expect(chart.line.points.map(\.mean) == [50])
    }

    @Test func aVolumeWithoutASizeHasNoChart() throws {
        var snapshot = keller()
        let id = volumeId(snapshot)
        let index = try #require(snapshot.volumes.firstIndex { $0.id == id })
        snapshot.volumes[index].totalBytes = nil
        let result = try content(snapshot, series: [.volumeUsed: [daily(id, [1, 2])]])
        #expect(result.volumes.isEmpty)
        #expect(result.isEmpty)
    }

    @Test func theForecastEndsAtOneHundredWhenItIsFullWithinTheRange() throws {
        let snapshot = keller()
        let id = volumeId(snapshot)
        let total = Double(try #require(NasEntry.capacityVolumes(of: snapshot)[0].totalBytes))
        let growth = total * 0.001
        let fullAt = Self.nowSeconds + 143 * day
        let trend = Forecast(daysToFull: 143, fullAt: fullAt, growthPerDay: growth)
        let used = daily(id, Array(repeating: total * 0.5, count: 10))

        let year = try #require(
            try content(snapshot, range: .year, series: [.volumeUsed: [used]], forecast: trend).volumes.first)
        #expect(year.forecast.count == 2)
        #expect(year.forecast.last?.percent == 100)
        #expect(year.forecast.last?.at == Date(timeIntervalSince1970: TimeInterval(fullAt)))
        let start = try #require(year.forecast.first)
        #expect(start.at == now)
        #expect(abs(start.percent - (100 - 0.1 * 143)) < 0.001)
        #expect(abs(try #require(year.growthPercentPerDay) - 0.1) < 0.001)
        #expect(year.forecastText?.main == "voll in 143 Tagen")
        #expect(year.historyDays == nil)
    }

    @Test func theForecastStopsAtTheEdgeOfTheRange() throws {
        let snapshot = keller()
        let id = volumeId(snapshot)
        let total = Double(try #require(NasEntry.capacityVolumes(of: snapshot)[0].totalBytes))
        let trend = Forecast(daysToFull: 143, fullAt: Self.nowSeconds + 143 * day, growthPerDay: total * 0.001)
        let used = daily(id, Array(repeating: total * 0.5, count: 10))

        let month = try #require(
            try content(snapshot, range: .month, series: [.volumeUsed: [used]], forecast: trend).volumes.first)
        let end = try #require(month.forecast.last)
        #expect(end.at == now.addingTimeInterval(30 * 86_400))
        #expect(end.percent < 100)
        // 30 days on the straight line from the start.
        #expect(abs(end.percent - (100 - 0.1 * 143 + 0.1 * 30)) < 0.001)
    }

    @Test func belowSevenDaysThereIsAHintInsteadOfAForecast() throws {
        let snapshot = keller()
        let id = volumeId(snapshot)
        let total = Double(try #require(NasEntry.capacityVolumes(of: snapshot)[0].totalBytes))
        let trend = Forecast(daysToFull: 10, fullAt: Self.nowSeconds + 10 * day, growthPerDay: 1)
        let used = daily(id, Array(repeating: total * 0.5, count: 6))
        let chart = try #require(
            try content(snapshot, series: [.volumeUsed: [used]], forecast: trend).volumes.first)
        #expect(chart.historyDays == 6)
        #expect(chart.forecast.isEmpty)
        #expect(chart.forecastText == nil)
    }

    @Test func daysOlderThanThirtyDaysDoNotCountForTheForecast() throws {
        let snapshot = keller()
        let id = volumeId(snapshot)
        let total = Double(try #require(NasEntry.capacityVolumes(of: snapshot)[0].totalBytes))
        // 40 days of values: only the last 30 count, which is more than 7.
        let used = daily(id, Array(repeating: total * 0.5, count: 40))
        let trend = Forecast(daysToFull: 10, fullAt: Self.nowSeconds + 10 * day, growthPerDay: 1)
        let chart = try #require(
            try content(snapshot, series: [.volumeUsed: [used]], forecast: trend).volumes.first)
        #expect(chart.historyDays == nil)
        #expect(chart.forecast.count == 2)
    }

    @Test func noTrendLeavesTheChartWithoutForecast() throws {
        let snapshot = keller()
        let id = volumeId(snapshot)
        let total = Double(try #require(NasEntry.capacityVolumes(of: snapshot)[0].totalBytes))
        let used = daily(id, Array(repeating: total * 0.5, count: 10))
        let chart = try #require(try content(snapshot, series: [.volumeUsed: [used]]).volumes.first)
        #expect(chart.forecast.isEmpty)
        #expect(chart.forecastText == nil)
        #expect(chart.historyDays == nil)
    }

    @Test func disksKeepTheNamesAndOrderOfTheSnapshot() throws {
        let snapshot = keller()
        let disks = snapshot.disks
        #expect(disks.count >= 2)
        let series = [
            HistorySeries(key: "unbekannt", points: [point(100, 40)]),
            HistorySeries(key: disks[1].id, points: [point(100, 41)]),
            HistorySeries(key: disks[0].id, points: [point(100, 42)]),
        ]
        let sys = HistorySeries(key: nil, points: [point(100, 50)])
        let result = try content(snapshot, series: [.diskTemp: series, .sysTemp: [sys]])
        #expect(result.temperatures.map(\.name) == [disks[0].name, disks[1].name, "unbekannt", "System"])
    }

    @Test func loadAndMemoryAreNamed() throws {
        let cpu = HistorySeries(key: nil, points: [point(100, 10)])
        let mem = HistorySeries(key: nil, points: [point(100, 60)])
        let result = try content(keller(), series: [.cpu: [cpu], .mem: [mem]])
        #expect(result.load.map(\.name) == ["CPU", "RAM"])
    }

    @Test func networkInMegabytesAndInterfaceWithoutTrafficIsLeftOut() throws {
        let busy = HistorySeries(key: "Adapter 1", points: [point(100, 2_000_000)])
        let quiet = HistorySeries(key: "Adapter 2", points: [point(100, 0)])
        let out = HistorySeries(key: "Adapter 1", points: [point(100, 500_000)])
        let result = try content(keller(), series: [.netRx: [busy, quiet], .netTx: [out, quiet]])
        #expect(result.networks.map(\.id) == ["Adapter 1"])
        let lines = try #require(result.networks.first).lines
        #expect(lines.map(\.name) == ["ein", "aus"])
        #expect(lines[0].points.map(\.mean) == [2])
        #expect(lines[1].points.map(\.mean) == [0.5])
    }

    @Test func withoutValuesThereIsNoHistory() throws {
        #expect(try content(keller(), series: [:]).isEmpty)
    }

    @Test func errorsOfTheCoreAreThrown() {
        struct Boom: Error {}
        #expect(throws: Boom.self) {
            try HistoryContent(
                snapshot: keller(), range: .month, now: now,
                load: { _, _ in throw Boom() }, forecast: { _ in nil })
        }
    }

    @Test func aFailingForecastIsNoForecast() throws {
        struct Boom: Error {}
        let snapshot = keller()
        let id = volumeId(snapshot)
        let total = Double(try #require(NasEntry.capacityVolumes(of: snapshot)[0].totalBytes))
        let used = daily(id, Array(repeating: total * 0.5, count: 10))
        let result = try HistoryContent(
            snapshot: snapshot, range: .month, now: now, load: loader([.volumeUsed: [used]]),
            forecast: { _ in throw Boom() })
        #expect(try #require(result.volumes.first).forecast.isEmpty)
    }

    @Test func rangesHaveLabelsAndLengths() {
        #expect(HistoryRange.options.map(\.label) == ["24 h", "7 Tage", "30 Tage", "1 Jahr"])
        #expect(HistoryRange.options.map(\.days) == [1, 7, 30, 365])
        #expect(HistoryRange(days: 30) == .month)
        #expect(HistoryRange(days: 3) == nil)
    }

    @Test func theDemoCoreGivesKellerAForecast() throws {
        let core = NasCore.demo(now: Self.nowSeconds)
        let snapshot = keller()
        let result = try HistoryContent(
            snapshot: snapshot, range: .month, now: now,
            load: { try core.history(nasId: "demo-keller", metric: $0, range: $1) },
            forecast: { try core.forecast(nasId: "demo-keller", volumeId: $0) })
        let chart = try #require(result.volumes.first { $0.id == "demo-keller-datavol1" })
        #expect(!chart.forecast.isEmpty)
        #expect(chart.historyDays == nil)
        #expect(chart.forecastText?.main == "voll in 143 Tagen")
        #expect(!result.temperatures.isEmpty)
    }
}
