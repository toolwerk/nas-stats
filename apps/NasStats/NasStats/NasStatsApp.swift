import SwiftUI

@main
struct NasStatsApp: App {
    /// State of every NAS; its accounts come from the keychain and are
    /// registered with the core.
    @State private var store: NasStore

    init() {
        let store = NasStore.live()
        _store = State(initialValue: store)
        // Started here and not with `.task`: the menu of the menu bar only
        // comes into being when it is opened.
        Task { await store.start() }
    }

    var body: some Scene {
        #if os(iOS)
        WindowGroup {
            ContentView()
                .environment(store)
        }
        #else
        // Menu bar only; LSUIElement keeps the app out of the Dock.
        MenuBarExtra("NAS Stats", systemImage: "externaldrive") {
            MenuBarView()
                .environment(store)
        }
        .menuBarExtraStyle(.menu)

        Window("NAS hinzufügen", id: "add-nas") {
            AddNasView()
                .environment(store)
        }
        .windowResizability(.contentSize)
        // Only from the menu, not at launch.
        .defaultLaunchBehavior(.suppressed)
        .restorationBehavior(.disabled)
        #endif
    }
}

extension StoreHealth {
    /// Traffic light colour of the provisional lists.
    var color: Color {
        switch self {
        case .critical: .red
        case .warning: .orange
        case .ok: .green
        case .unknown: .gray
        }
    }

    /// Symbol for the menu, where colours do not show.
    var symbol: String {
        switch self {
        case .critical: "xmark.octagon"
        case .warning: "exclamationmark.triangle"
        case .ok: "checkmark.circle"
        case .unknown: "questionmark.circle"
        }
    }
}
