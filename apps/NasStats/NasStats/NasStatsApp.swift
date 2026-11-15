import SwiftUI

@main
struct NasStatsApp: App {
    #if os(iOS)
    /// State of every NAS; its accounts come from the keychain and are
    /// registered with the core.
    @State private var store: NasStore

    init() {
        let store = NasStore.live()
        _store = State(initialValue: store)
        Task { await store.start() }
    }
    #else
    /// Store and menu bar symbol live in the delegate, for as long as the
    /// app runs (#27).
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #endif

    var body: some Scene {
        #if os(iOS)
        WindowGroup {
            ContentView()
                .environment(store)
        }
        #else
        // Menu bar only; LSUIElement keeps the app out of the Dock. The
        // status item and its windows come from `StatusItemController`.
        Settings {
            EmptyView()
        }
        .commands {
            // ⌘, would open this empty scene instead of the settings window
            // of `StatusItemController`.
            CommandGroup(replacing: .appSettings) {}
        }
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
