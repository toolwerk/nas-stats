import Foundation
import Testing

/// Colour, text and tooltip of the menu bar symbol (#27).
struct MenuBarSummaryTests {
    private let de = Locale(identifier: "de_DE")

    /// An entry with a snapshot of the given health and one volume per fill
    /// level; without health no snapshot.
    private func entry(_ id: String, health: Health?, fills: [Double?] = []) -> NasEntry {
        let base = NasEntry(id: id, baseUrl: "https://\(id).invalid")
        guard let health else { return base }
        var snapshot = demoSnapshot()
        snapshot.nas.id = id
        snapshot.demo = false
        snapshot.worstHealth = health
        let template = snapshot.volumes[0]
        snapshot.volumes = fills.map { fill in
            var volume = template
            volume.usedPercent = fill
            return volume
        }
        var entry = base
        entry.snapshot = snapshot
        return entry
    }

    @Test func emptyHasNoHealthAndNoColour() {
        let summary = MenuBarSummary(entries: [], demo: false)
        #expect(summary.health == nil)
        #expect(summary.symbolLevel == nil)
        #expect(summary.maxFillPercent == nil)
        #expect(summary.text(for: .fill, locale: de) == nil)
        #expect(summary.text(for: .warnings, locale: de) == nil)
        #expect(summary.tooltip == "NAS Stats")
    }

    @Test func worstNasWins() {
        let summary = MenuBarSummary(
            entries: [entry("a", health: .ok), entry("b", health: .warning), entry("c", health: nil)],
            demo: false
        )
        #expect(summary.health == .warning)
        #expect(summary.symbolLevel == .warning)
        #expect(summary.tooltip == "NAS Stats – Warnung")

        let critical = MenuBarSummary(
            entries: [entry("a", health: .warning), entry("b", health: .critical)], demo: false
        )
        #expect(critical.symbolLevel == .critical)

        let ok = MenuBarSummary(entries: [entry("a", health: .ok)], demo: false)
        #expect(ok.symbolLevel == .ok)
    }

    @Test func unknownStaysMonochrome() {
        let summary = MenuBarSummary(entries: [entry("a", health: nil)], demo: false)
        #expect(summary.health == .unknown)
        #expect(summary.symbolLevel == nil)
    }

    @Test func fillIsHighestVolumeRoundedDown() {
        let summary = MenuBarSummary(
            entries: [
                entry("a", health: .ok, fills: [42.0, nil]),
                entry("b", health: .ok, fills: [87.9]),
            ],
            demo: false
        )
        #expect(summary.maxFillPercent == 87.9)
        #expect(summary.text(for: .fill, locale: de) == "87 %")
        #expect(summary.text(for: .symbol, locale: de) == nil)
    }

    @Test func fillWithoutVolumesIsNil() {
        let summary = MenuBarSummary(
            entries: [entry("a", health: .ok), entry("b", health: .ok, fills: [nil])], demo: false
        )
        #expect(summary.maxFillPercent == nil)
        #expect(summary.text(for: .fill, locale: de) == nil)
    }

    @Test func warningsCountWarningAndCritical() {
        let none = MenuBarSummary(entries: [entry("a", health: .ok)], demo: false)
        #expect(none.warningCount == 0)
        #expect(none.text(for: .warnings, locale: de) == nil)

        let two = MenuBarSummary(
            entries: [
                entry("a", health: .ok),
                entry("b", health: .warning),
                entry("c", health: .critical),
                entry("d", health: nil),
            ],
            demo: false
        )
        #expect(two.warningCount == 2)
        #expect(two.text(for: .warnings, locale: de) == "2")
    }

    @Test func demoTooltip() {
        let summary = MenuBarSummary(entries: [entry("a", health: .warning)], demo: true)
        #expect(summary.tooltip == "NAS Stats – Demo-Modus")
    }

    @Test func displayFromDefaults() {
        #expect(MenuBarDisplay(stored: nil) == .symbol)
        #expect(MenuBarDisplay(stored: "kaputt") == .symbol)
        #expect(MenuBarDisplay(stored: "fill") == .fill)
        #expect(MenuBarDisplay.allCases.map(\.title)
            == ["Symbol", "Symbol und Füllstand", "Symbol und Warnungen"])
    }
}
