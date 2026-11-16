import Foundation
import Testing

struct WidgetContentTests {
    let now = Date(timeIntervalSince1970: 1_800_000_000)
    let locale = Locale(identifier: "de_DE")
    let zone = TimeZone(identifier: "UTC")!

    func volume(_ name: String, _ percent: Double?, forecast: FillForecast? = nil) -> WidgetVolume {
        WidgetVolume(
            id: name, name: name, usedPercent: percent, usedBytes: nil, totalBytes: nil,
            forecast: forecast)
    }

    func nas(
        finding: String? = nil, problem: String? = nil, volumes: [WidgetVolume] = [],
        observedAgo: TimeInterval? = 60
    ) -> WidgetNas {
        WidgetNas(
            id: "a", name: "Keller", model: nil, vendor: nil, status: .ok,
            firstFinding: finding, firstFindingLevel: finding == nil ? nil : .critical,
            findingCount: finding == nil ? 0 : 1, problemLine: problem, volumes: volumes,
            hottestDiskC: nil, cpuPercent: nil, memoryPercent: nil, lastBackupAt: nil,
            observedAt: observedAgo.map { now.addingTimeInterval(-$0) }, fullestVolumeHistory: [])
    }

    func snapshot(_ nas: WidgetNas, demo: Bool = false) -> WidgetSnapshot {
        WidgetSnapshot(
            writtenAt: now, demo: demo,
            staleAfterSeconds: WidgetSnapshot.staleAfter(intervalMinutes: 5),
            thresholds: .standard, nas: [nas])
    }

    func lines(_ nas: WidgetNas, demo: Bool = false) -> [WidgetLine] {
        nas.smallLines(in: snapshot(nas, demo: demo), now: now, locale: locale, timeZone: zone)
    }

    @Test func theFullestVolumeIsTheOneWithTheHighestPercent() {
        let n = nas(volumes: [volume("a", 40), volume("b", 87), volume("c", 64)])
        #expect(n.fullestVolume?.id == "b")
    }

    @Test func aTieGoesToTheFirstVolumeAndMissingPercentsDoNotCount() {
        #expect(nas(volumes: [volume("a", 50), volume("b", 50)]).fullestVolume?.id == "a")
        #expect(nas(volumes: [volume("a", nil), volume("b", 10)]).fullestVolume?.id == "b")
        #expect(nas(volumes: [volume("a", nil)]).fullestVolume == nil)
        #expect(nas().fullestVolume == nil)
    }

    @Test func theMediumWidgetShowsTheThreeFullestVolumesWithoutPercentLast() {
        let n = nas(volumes: [
            volume("n", nil), volume("a", 10), volume("b", 90), volume("c", 90), volume("d", 50),
            volume("e", 70),
        ])
        #expect(n.mediumVolumes.map(\.id) == ["b", "c", "e"])
        let few = nas(volumes: [volume("n", nil), volume("a", 10)])
        #expect(few.mediumVolumes.map(\.id) == ["a", "n"])
        #expect(nas().mediumVolumes.isEmpty)
    }

    @Test func theErrorComesBeforeTheFirstWarning() {
        let both = nas(finding: "Platte 3 bei 52 °C", problem: "Keine Verbindung")
        #expect(both.messageLine == WidgetLine(text: "Keine Verbindung", style: .problem))
        let finding = nas(finding: "Platte 3 bei 52 °C")
        #expect(finding.messageLine == WidgetLine(text: "Platte 3 bei 52 °C", style: .finding(.critical)))
        #expect(nas().messageLine == nil)
    }

    @Test func theForecastFollowsTheFullestVolumeAndNoTrendShowsNothing() {
        let full = FillForecast.full(days: 143, date: nil)
        let n = nas(volumes: [volume("a", 87, forecast: full), volume("b", 95, forecast: nil)])
        // The fullest volume has no forecast.
        #expect(lines(n).isEmpty)
        let m = nas(volumes: [volume("a", 87, forecast: full), volume("b", 60, forecast: .noTrend)])
        #expect(lines(m) == [WidgetLine(text: "voll in 143 Tagen", style: .forecast)])
        let noTrend = nas(volumes: [volume("a", 87, forecast: .noTrend)])
        #expect(lines(noTrend).isEmpty)
    }

    @Test func aWarningComesFirstThenTheForecast() {
        let n = nas(
            finding: "Volume DataVol1 zu 87 % belegt",
            volumes: [volume("a", 87, forecast: .full(days: 143, date: nil))])
        #expect(lines(n).map(\.style) == [.finding(.critical), .forecast])
    }

    @Test func anOldStateReplacesTheForecastBelowAWarning() {
        let n = nas(
            finding: "Platte 3 bei 52 °C",
            volumes: [volume("a", 87, forecast: .full(days: 143, date: nil))], observedAgo: 3 * 3600)
        let result = lines(n)
        #expect(result.map(\.style) == [.finding(.critical), .stale])
        #expect(result.last?.text.hasPrefix("Stand ") == true)
    }

    @Test func anOldStateComesAfterTheForecastWithoutAWarning() {
        let n = nas(
            volumes: [volume("a", 87, forecast: .full(days: 143, date: nil))], observedAgo: 3 * 3600)
        #expect(lines(n).map(\.style) == [.forecast, .stale])
    }

    @Test func aFreshStateAndDemoValuesNeverShowStand() {
        let fresh = nas(finding: "x", observedAgo: 60)
        #expect(lines(fresh).map(\.style) == [.finding(.critical)])
        let old = nas(finding: "x", observedAgo: 3 * 3600)
        #expect(lines(old, demo: true).map(\.style) == [.finding(.critical)])
    }

    @Test func aNasWithoutAStateHasNoStandLine() {
        let n = nas(problem: "Noch keine Abfrage", observedAgo: nil)
        #expect(lines(n).map(\.style) == [.problem])
    }

    // MARK: Widgets „Füllstand“ und „Alle NAS“ (#38)

    func sized(_ name: String, _ percent: Double?, used: UInt64?, total: UInt64?) -> WidgetVolume {
        WidgetVolume(
            id: name, name: name, usedPercent: percent, usedBytes: used, totalBytes: total,
            forecast: nil)
    }

    func status(_ nas: WidgetNas, demo: Bool = false) -> WidgetLine? {
        nas.rowStatus(in: snapshot(nas, demo: demo), now: now, locale: locale, timeZone: zone)
    }

    @Test func theCapacityLineSumsTheVolumesWithBothSizes() {
        let tb: UInt64 = 1_099_511_627_776
        let n = nas(volumes: [
            sized("a", 50, used: 5 * tb, total: 10 * tb),
            sized("b", 40, used: 4 * tb, total: 10 * tb),
            sized("c", nil, used: nil, total: 10 * tb),
        ])
        #expect(n.capacityLine(locale: locale) == "9,0 von 20,0 TB")
        #expect(nas(volumes: [volume("a", 50)]).capacityLine(locale: locale) == nil)
        #expect(nas().capacityLine(locale: locale) == nil)
    }

    @Test func anOldStateComesFirstInARow() {
        let old = nas(finding: "x", problem: "Nicht erreichbar", observedAgo: 3 * 3600)
        #expect(status(old)?.style == .stale)
        #expect(status(old)?.text.hasPrefix("Stand ") == true)
        // Demo values never turn old.
        #expect(status(old, demo: true)?.style == .problem)
    }

    @Test func theErrorComesBeforeTheWarningsInARow() {
        let n = nas(finding: "x", problem: "Nicht erreichbar")
        #expect(status(n) == WidgetLine(text: "Nicht erreichbar", style: .problem))
    }

    @Test func aRowCountsTheWarningsInSingularAndPlural() {
        var n = nas(finding: "x")
        #expect(status(n) == WidgetLine(text: "1 Warnung", style: .finding(.critical)))
        n.findingCount = 3
        #expect(status(n) == WidgetLine(text: "3 Warnungen", style: .finding(.critical)))
        n.firstFindingLevel = nil
        #expect(status(n)?.style == .finding(.warning))
        #expect(status(nas()) == nil)
    }

    @Test func theRowDetailNamesTheFullestVolumeAndTheBackup() {
        var n = nas(volumes: [volume("a", 40), volume("DataVol1", 87.9)])
        n.lastBackupAt = now.addingTimeInterval(-9 * 3600)
        #expect(n.rowDetail(now: now, locale: locale) == "DataVol1 87 % · Backup vor 9 h")
        n.lastBackupAt = nil
        #expect(n.rowDetail(now: now, locale: locale) == "DataVol1 87 % · Backup n/a")
        let without = nas(volumes: [volume("a", nil)])
        #expect(without.rowDetail(now: now, locale: locale) == "Backup n/a")
    }

    func manySnapshot(_ count: Int, percents: [Double?] = []) -> WidgetSnapshot {
        let all = (0 ..< count).map { index -> WidgetNas in
            var n = nas(volumes: percents.indices.contains(index) ? [volume("v\(index)", percents[index])] : [])
            n.id = "n\(index)"
            return n
        }
        return WidgetSnapshot(
            writtenAt: now, demo: false, staleAfterSeconds: 1800, thresholds: .standard, nas: all)
    }

    @Test func theLargeWidgetShowsThreeNasAndCountsTheRest() {
        for (count, shown, hidden) in [(0, 0, 0), (2, 2, 0), (3, 3, 0), (5, 3, 2)] {
            let s = manySnapshot(count)
            #expect(s.largeRows.count == shown)
            #expect(s.largeHiddenCount == hidden)
        }
        #expect(manySnapshot(5).largeRows.map(\.id) == ["n0", "n1", "n2"])
    }

    @Test func theHistoryBelongsToTheFullestVolumeOfAllNas() {
        let s = manySnapshot(3, percents: [40, 87, 60])
        #expect(s.historySource?.nas.id == "n1")
        #expect(s.historySource?.volume.id == "v1")
        // A tie goes to the first NAS; the fullest NAS may be a hidden one.
        #expect(manySnapshot(2, percents: [50, 50]).historySource?.nas.id == "n0")
        #expect(manySnapshot(5, percents: [10, 10, 10, 10, 99]).historySource?.nas.id == "n4")
        #expect(manySnapshot(2, percents: [nil, nil]).historySource == nil)
        #expect(manySnapshot(0).historySource == nil)
    }
}

