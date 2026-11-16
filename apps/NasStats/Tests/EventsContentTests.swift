import Foundation
import Testing

/// List of the tab „Ereignisse“ (#20).
struct EventsContentTests {
    private static let zone = TimeZone(identifier: "Europe/Berlin")!
    private static let locale = Locale(identifier: "de_DE")

    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = Self.zone
        return calendar
    }

    private func at(_ year: Int, _ month: Int, _ day: Int, _ hour: Int, _ minute: Int) -> Int64 {
        let parts = DateComponents(
            calendar: calendar, timeZone: Self.zone,
            year: year, month: month, day: day, hour: hour, minute: minute
        )
        return Int64(parts.date!.timeIntervalSince1970)
    }

    /// 16 November 2026, 14:00.
    private var now: Date { Date(timeIntervalSince1970: TimeInterval(at(2026, 11, 16, 14, 0))) }

    private func event(
        _ nas: String, _ kind: EventKind, _ text: String, start: Int64, minutes: Int64? = nil,
        level: Health = .warning
    ) -> NasEvent {
        NasEvent(
            nasId: nas, kind: kind, subject: nil, level: level, text: text,
            startedAt: start, endedAt: minutes.map { start + $0 * 60 }
        )
    }

    /// Open disk temperature today, two resolved ones yesterday, an open
    /// volume event on 12 November and an old backup problem last year.
    private var events: [NasEvent] {
        [
            event("keller", .upsOnBattery, "USV auf Batterie", start: at(2026, 11, 15, 6, 31), minutes: 4),
            event("keller", .volumeFill, "DataVol1 bei 87 %", start: at(2026, 11, 12, 22, 15)),
            event("keller", .diskTemperature, "Platte 3 bei 52 °C", start: at(2026, 11, 16, 13, 35)),
            event("homelab", .unreachable, "Homelab nicht erreichbar", start: at(2026, 11, 15, 3, 10), minutes: 18),
            event("buero", .backupFailed, "Backup fehlgeschlagen", start: at(2025, 12, 3, 8, 0), minutes: 90, level: .critical),
        ]
    }

    private let names = ["keller": "Keller", "homelab": "Homelab", "buero": "Büro"]

    private func content(_ filter: EventFilter = EventFilter(), events: [NasEvent]? = nil) -> EventsContent {
        EventsContent(
            events: events ?? self.events, names: names, filter: filter,
            now: now, calendar: calendar, locale: Self.locale
        )
    }

    @Test func groupsByDayNewestFirst() {
        let days = content().days
        #expect(days.map(\.title) == ["Heute", "Gestern", "12. November", "3. Dezember 2025"])
        #expect(days.map { $0.rows.map(\.title) } == [
            ["Platte 3 bei 52 °C"],
            ["USV auf Batterie", "Homelab nicht erreichbar"],
            ["DataVol1 bei 87 %"],
            ["Backup fehlgeschlagen"],
        ])
    }

    @Test func rowShowsTimeNasAndDuration() {
        let days = content().days
        let open = days[0].rows[0]
        #expect(open.time == "13:35")
        #expect(open.detail == "Keller · offen seit 25 min")
        #expect(!open.resolved)
        let resolved = days[1].rows[0]
        #expect(resolved.time == "06:31")
        #expect(resolved.detail == "Keller · erledigt nach 4 min")
        #expect(resolved.resolved)
        #expect(days[2].rows[0].detail == "Keller · offen seit 3 Tage")
        #expect(days[3].rows[0].detail == "Büro · erledigt nach 1 h")
    }

    @Test func levelAndSymbolFollowKindAndLevel() {
        let days = content().days
        #expect(days[0].rows[0].level == .warning)
        #expect(days[0].rows[0].symbol == "internaldrive")
        #expect(days[1].rows[1].level == .offline)
        #expect(days[1].rows[1].symbol == "wifi.slash")
        #expect(days[3].rows[0].level == .critical)
        #expect(days[3].rows[0].symbol == "clock.arrow.circlepath")
    }

    @Test func rowIdIsTheIdOfTheNotification() {
        let source = events[2]
        #expect(content().days[0].rows[0].id == EventNotifier.id(of: source))
    }

    @Test func filterByNas() {
        let filtered = content(EventFilter(nasId: "keller"))
        #expect(filtered.days.flatMap(\.rows).count == 3)
        #expect(filtered.days.map(\.title) == ["Heute", "Gestern", "12. November"])
    }

    @Test func filterByKindCoversSeveralEventKinds() {
        let volumes = content(EventFilter(category: .volumes))
        #expect(volumes.days.flatMap(\.rows).map(\.title) == ["DataVol1 bei 87 %"])
        let disks = content(EventFilter(category: .disks))
        #expect(disks.days.flatMap(\.rows).map(\.title) == ["Platte 3 bei 52 °C"])
        let fans = content(EventFilter(category: .fans))
        #expect(fans.days.isEmpty)
    }

    @Test func filterOnlyOpen() {
        let open = content(EventFilter(onlyOpen: true))
        #expect(open.days.flatMap(\.rows).map(\.title) == ["Platte 3 bei 52 °C", "DataVol1 bei 87 %"])
    }

    @Test func filtersCombine() {
        let none = content(EventFilter(nasId: "homelab", category: .unreachable, onlyOpen: true))
        #expect(none.days.isEmpty)
        let one = content(EventFilter(nasId: "keller", category: .ups))
        #expect(one.days.flatMap(\.rows).map(\.title) == ["USV auf Batterie"])
    }

    @Test func emptyFilterResultStillKnowsThereAreEvents() {
        let filtered = content(EventFilter(nasId: "unknown"))
        #expect(filtered.days.isEmpty)
        #expect(filtered.hasAnyEvents)
        let empty = content(events: [])
        #expect(empty.days.isEmpty)
        #expect(!empty.hasAnyEvents)
    }

    @Test func unknownNasHasAFallbackName() {
        let lost = event("gone", .fanHealth, "Lüfter 2 steht", start: at(2026, 11, 16, 9, 0))
        let rows = content(events: [lost]).days[0].rows
        #expect(rows[0].detail.hasPrefix("Unbekanntes NAS · "))
    }

    @Test func eventStartedInTheFutureCountsZero() {
        let early = event("keller", .alert, "Warnung", start: Int64(now.timeIntervalSince1970) + 600)
        #expect(content(events: [early]).days[0].rows[0].detail == "Keller · offen seit 0 min")
    }

    @Test func sameStartKeepsAStableOrder() {
        let start = at(2026, 11, 16, 12, 0)
        let first = event("keller", .fanHealth, "Lüfter", start: start)
        let second = event("keller", .alert, "Warnung", start: start)
        let one = content(events: [first, second]).days[0].rows.map(\.id)
        let other = content(events: [second, first]).days[0].rows.map(\.id)
        #expect(one == other)
    }
}
