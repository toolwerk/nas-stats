import SwiftUI

@main
struct NasStatsApp: App {
    #if os(iOS)
    /// State of every NAS; its accounts come from the keychain and are
    /// registered with the core.
    @State private var store: NasStore
    @Environment(\.scenePhase) private var scenePhase

    init() {
        let store = NasStore.live()
        _store = State(initialValue: store)
        store.start()
    }
    #else
    /// Store and menu bar symbol live in the delegate, for as long as the
    /// app runs (#27).
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #endif

    var body: some Scene {
        #if os(iOS)
        WindowGroup {
            TabView {
                Tab("Übersicht", systemImage: "externaldrive") {
                    OverviewView()
                }
                Tab("Ereignisse", systemImage: "bell") {
                    EventsView()
                }
                Tab("Einstellungen", systemImage: "gearshape") {
                    SettingsView()
                }
            }
            .environment(store)
        }
        // Back in the app, due NAS are queried at once (the planner of the
        // core sleeps while the app is suspended); leaving it asks iOS for
        // an occasional query in the background.
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .active:
                Task { await store.refreshAll() }
            case .background:
                let minutes = store.queryIntervalMinutes
                Task { await BackgroundRefresh.schedule(intervalMinutes: minutes) }
            default:
                break
            }
        }
        .backgroundTask(.appRefresh(BackgroundRefresh.identifier)) { [store] in
            // The next run first, so one that ends early still has a successor.
            let minutes = await MainActor.run { store.queryIntervalMinutes }
            await BackgroundRefresh.schedule(intervalMinutes: minutes)
            await store.refreshAll()
        }
        #else
        // Menu bar only; LSUIElement keeps the app out of the Dock. The
        // status item and its windows (also the settings, #21) come from
        // `StatusItemController`.
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
    /// Traffic light colour of the settings list.
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
