import Foundation

/// What the menu bar shows next to the NAS symbol (#27).
enum MenuBarDisplay: String, CaseIterable, Sendable {
    /// Only the symbol in the colour of the worst NAS.
    case symbol
    /// Symbol and the highest fill level of all volumes.
    case fill
    /// Symbol and the number of NAS with a warning or critical.
    case warnings

    /// Key in the standard defaults.
    static let defaultsKey = "menuBarDisplay"

    var title: String {
        switch self {
        case .symbol: "Symbol"
        case .fill: "Symbol und Füllstand"
        case .warnings: "Symbol und Warnungen"
        }
    }

    /// The stored choice; missing or unknown is `.symbol`.
    init(stored: String?) {
        self = stored.flatMap(Self.init(rawValue:)) ?? .symbol
    }
}

/// State of all NAS condensed for the menu bar symbol. Without AppKit, so
/// the tests cover it.
struct MenuBarSummary: Equatable, Sendable {
    /// Worst health of all NAS; `nil` without NAS.
    let health: StoreHealth?
    /// Highest fill level of all volumes in percent; `nil` when none is known.
    let maxFillPercent: Double?
    /// NAS with a warning or critical.
    let warningCount: Int
    /// The entries are demo values.
    let demo: Bool

    init(entries: [NasEntry], demo: Bool) {
        health = entries.map(\.health).min()
        maxFillPercent = entries
            .flatMap { $0.snapshot?.volumes ?? [] }
            .compactMap(\.usedPercent)
            .max()
        warningCount = entries.count { $0.health <= .warning }
        self.demo = demo
    }

    /// Level of the symbol colour; `nil` keeps the symbol monochrome
    /// (no NAS, state unknown).
    var symbolLevel: StatusLevel? {
        switch health {
        case .critical: .critical
        case .warning: .warning
        case .ok: .ok
        case .unknown, nil: nil
        }
    }

    /// Text next to the symbol; `nil` shows the symbol alone.
    func text(for display: MenuBarDisplay, locale: Locale = .current) -> String? {
        switch display {
        case .symbol:
            return nil
        case .fill:
            guard let maxFillPercent else { return nil }
            // Rounded down, like the fill bars: 87.9 is "87 %".
            return StatusFormat.percent(
                StatusFormat.floored(maxFillPercent, fractionDigits: 0),
                fractionDigits: 0,
                locale: locale
            )
        case .warnings:
            return warningCount > 0 ? String(warningCount) : nil
        }
    }

    /// Tooltip and accessibility label of the symbol.
    var tooltip: String {
        if demo {
            return "NAS Stats – Demo-Modus"
        }
        guard let health else { return "NAS Stats" }
        return "NAS Stats – \(health.title)"
    }
}
