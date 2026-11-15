import Foundation
import Testing

/// Head of the overview and the values of the cards (#17).
@MainActor
struct OverviewTests {
    private static let now: Int64 = 1_794_665_100

    /// An entry with a snapshot of the given health and findings, observed
    /// `minutesAgo` before `now`.
    private func entry(
        _ id: String,
        health: Health? = .ok,
        findings: [Finding] = [],
        minutesAgo: Int64 = 0
    ) -> NasEntry {
        var snapshot = demoSnapshot()
        snapshot.nas.id = id
        snapshot.demo = false
        snapshot.worstHealth = health
        snapshot.findings = findings
        snapshot.observedAt = Self.now - minutesAgo * 60
        return NasEntry(id: id, baseUrl: "https://\(id).invalid", snapshot: snapshot)
    }

    private func problem(_ kind: ProblemKind, stops: Bool = false) -> NasProblem {
        NasProblem(
            kind: kind, message: "NAS antwortet nicht", stopsPolling: stops,
            oldFingerprint: nil, newFingerprint: nil, httpStatus: nil
        )
    }

    private let warm = Finding(kind: .diskTemperature, level: .warning, subject: nil, text: "Platte 3 bei 52 °C")

    @Test func allHealthy() {
        #expect(OverviewSummary.sentence([entry("a"), entry("b"), entry("c")], demo: false) == "Alle 3 NAS gesund")
        #expect(OverviewSummary.sentence([entry("a")], demo: false) == "1 NAS gesund")
    }

    @Test func mixedStatesInTheOrderOfTheirWeight() {
        var offline = entry("d")
        offline.problem = problem(.unreachable)
        var blocked = entry("e")
        blocked.problem = problem(.loginFailed, stops: true)
        blocked.blocked = true
        let entries = [
            entry("a"), entry("b", findings: [warm]), offline, blocked,
            NasEntry(id: "f", baseUrl: "https://f.invalid"),
        ]
        #expect(entries.map(\.statusLevel) == [.ok, .warning, .offline, .critical, .unknown])
        #expect(
            OverviewSummary.sentence(entries, demo: false)
                == "5 NAS: 1 kritisch, 1 mit Warnung, 1 nicht erreichbar, 1 unbekannt, 1 gesund"
        )
    }

    @Test func empty() {
        #expect(OverviewSummary.sentence([], demo: false) == "Noch kein NAS")
        #expect(OverviewSummary.stamp([], now: Date()) == "Noch kein Stand")
        #expect(OverviewSummary.stamp([NasEntry(id: "a", baseUrl: "https://a.invalid")]) == "Noch kein Stand")
    }

    @Test func demoIsMarked() {
        let entries = [entry("a", findings: [warm]), entry("b"), entry("c")]
        #expect(OverviewSummary.sentence(entries, demo: true) == "Demo-Daten · 3 NAS: 1 mit Warnung, 2 gesund")
    }

    @Test func stampTakesTheOldestState() {
        let now = Date(timeIntervalSince1970: TimeInterval(Self.now))
        let entries = [entry("a", minutesAgo: 1), entry("b", minutesAgo: 7), NasEntry(id: "c", baseUrl: "https://c.invalid")]
        #expect(OverviewSummary.stamp(entries, now: now) == "Stand vor 7 min")
        #expect(OverviewSummary.stamp([entry("a")], now: now) == "Stand gerade eben")
    }

    @Test func offlineOnlyWithoutBlock() {
        var timeout = entry("a")
        timeout.problem = problem(.timeout)
        #expect(timeout.statusLevel == .offline)
        #expect(timeout.health == .warning)
        timeout.blocked = true
        #expect(timeout.statusLevel == .critical)
        #expect(timeout.problemLine == "NAS antwortet nicht")
    }

    @Test func aCriticalNasStaysCriticalWhenItStopsAnswering() {
        let battery = Finding(kind: .upsOnBattery, level: .critical, subject: nil, text: "USV auf Batterie")
        var e = entry("a", findings: [battery])
        e.problem = problem(.unreachable)
        #expect(e.health == .critical)
        #expect(e.statusLevel == .critical)
        #expect(OverviewSummary.sentence([e], demo: false) == "1 NAS: 1 kritisch")
    }

    @Test func problemLinePrefersTheLocalError() {
        var e = entry("a")
        #expect(e.problemLine == nil)
        e.queryError = "Abfrage abgebrochen"
        #expect(e.problemLine == "Abfrage abgebrochen")
        e.problem = problem(.http)
        #expect(e.problemLine == "NAS antwortet nicht")
        e.localError = "Anbieter unbekannt"
        #expect(e.problemLine == "Anbieter unbekannt")
    }

    @Test func cardValuesOfTheDemoKeller() throws {
        let keller = try #require(demoSnapshots(now: Self.now).first)
        let e = NasEntry(id: keller.nas.id, baseUrl: keller.nas.baseUrl, provider: keller.nas.provider, snapshot: keller)
        #expect(e.statusLevel == .warning)
        #expect(e.vendorLabel == "QNAP")
        #expect(e.capacityVolumes.map(\.name) == ["DataVol1"])
        #expect(e.hottestDiskC == 52)
        // The offsite backup 9 h ago, not the snapshot task 5 minutes ago.
        #expect(e.lastBackupAt == Date(timeIntervalSince1970: TimeInterval(Self.now - 9 * 3600)))
    }

    @Test func capacityVolumesFallBackToPools() throws {
        let homelab = try #require(demoSnapshots(now: Self.now).last)
        let e = NasEntry(id: homelab.nas.id, baseUrl: homelab.nas.baseUrl, provider: homelab.nas.provider, snapshot: homelab)
        #expect(e.capacityVolumes.map(\.name) == ["tank", "apps"])
        #expect(e.vendorLabel == "TrueNAS")
        #expect(NasEntry(id: "x", baseUrl: "https://x.invalid").capacityVolumes.isEmpty)
    }
}
