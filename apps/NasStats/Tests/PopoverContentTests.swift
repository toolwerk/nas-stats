import Foundation
import Testing

/// Choice of the NAS, event line and footer of the Mac popover (#28).
struct PopoverContentTests {
    private static let now: Int64 = 1_794_665_100
    private var nowDate: Date { Date(timeIntervalSince1970: TimeInterval(Self.now)) }

    /// An entry with a snapshot observed `minutesAgo` before `now`; without
    /// minutes no snapshot.
    private func entry(_ id: String, minutesAgo: Int64? = nil) -> NasEntry {
        let base = NasEntry(id: id, baseUrl: "https://\(id).invalid")
        guard let minutesAgo else { return base }
        var snapshot = demoSnapshot()
        snapshot.nas.id = id
        snapshot.demo = false
        snapshot.observedAt = Self.now - minutesAgo * 60
        return NasEntry(id: id, baseUrl: base.baseUrl, snapshot: snapshot)
    }

    private func event(
        _ nas: String, _ kind: EventKind = .diskTemperature, _ text: String = "Platte 3 bei 52 °C",
        minutesAgo: Int64, ended: Bool = false, subject: String? = nil
    ) -> NasEvent {
        let start = Self.now - minutesAgo * 60
        return NasEvent(
            nasId: nas, kind: kind, subject: subject, level: .warning, text: text,
            startedAt: start, endedAt: ended ? start + 60 : nil
        )
    }

    // MARK: - Choice of the NAS

    @Test func chosenNasStaysWhilePresent() {
        let entries = [entry("keller"), entry("buero"), entry("homelab")]
        #expect(PopoverContent.shownId(entries: entries, chosen: "buero") == "buero")
    }

    @Test func withoutChoiceTheFirstAndWorst() {
        let entries = [entry("keller"), entry("buero")]
        #expect(PopoverContent.shownId(entries: entries, chosen: nil) == "keller")
    }

    @Test func removedChoiceFallsBackToTheFirst() {
        let entries = [entry("keller"), entry("homelab")]
        #expect(PopoverContent.shownId(entries: entries, chosen: "buero") == "keller")
    }

    @Test func withoutNasNothingIsShown() {
        #expect(PopoverContent.shownId(entries: [], chosen: nil) == nil)
        #expect(PopoverContent.shownId(entries: [], chosen: "keller") == nil)
    }

    // MARK: - Latest open event

    @Test func latestOpenEventOfTheNas() {
        let events = [
            event("keller", .volumeFill, "DataVol1 bei 87 %", minutesAgo: 600),
            event("keller", minutesAgo: 25),
            event("keller", .upsOnBattery, "USV auf Batterie", minutesAgo: 5, ended: true),
            event("buero", .fanHealth, "Lüfter 1 steht", minutesAgo: 1),
        ]
        #expect(PopoverContent.latestOpenEvent(events, nasId: "keller")?.text == "Platte 3 bei 52 °C")
        #expect(PopoverContent.latestOpenEvent(events, nasId: "buero")?.text == "Lüfter 1 steht")
    }

    @Test func endedOrForeignEventsDoNotCount() {
        let events = [
            event("keller", minutesAgo: 25, ended: true),
            event("buero", minutesAgo: 3),
        ]
        #expect(PopoverContent.latestOpenEvent(events, nasId: "keller") == nil)
        #expect(PopoverContent.latestOpenEvent([], nasId: "keller") == nil)
    }

    @Test func sameStartTakesTheSmallerId() {
        let disk4 = event("keller", .diskTemperature, "Platte 4 bei 53 °C", minutesAgo: 10, subject: "4")
        let disk3 = event("keller", .diskTemperature, "Platte 3 bei 52 °C", minutesAgo: 10, subject: "3")
        #expect(PopoverContent.latestOpenEvent([disk4, disk3], nasId: "keller")?.subject == "3")
        #expect(PopoverContent.latestOpenEvent([disk3, disk4], nasId: "keller")?.subject == "3")
    }

    @Test func openSinceLikeTheEventsTab() {
        let open = event("keller", minutesAgo: 25)
        #expect(PopoverContent.openSince(open, now: nowDate) == "offen seit 25 min")
    }

    // MARK: - Footer

    @Test func footerShowsTheAgeOfTheShownNas() {
        #expect(PopoverContent.footer(entry: entry("keller", minutesAgo: 2), demo: false, now: nowDate)
            == "Stand vor 2 min")
        #expect(PopoverContent.footer(entry: entry("keller"), demo: false, now: nowDate) == "Noch kein Stand")
        #expect(PopoverContent.footer(entry: nil, demo: false, now: nowDate) == "Noch kein Stand")
    }

    @Test func footerMarksDemoValues() {
        #expect(PopoverContent.footer(entry: entry("keller", minutesAgo: 2), demo: true, now: nowDate)
            == "Demo · Stand vor 2 min")
    }
}
