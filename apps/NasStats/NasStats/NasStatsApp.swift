import SwiftUI

@main
struct NasStatsApp: App {
    #if os(iOS)
    /// State of every NAS; its accounts come from the keychain and are
    /// registered with the core.
    @State private var store: NasStore
    @Environment(\.scenePhase) private var scenePhase
    /// Takes the silent push of iCloud (#37).
    @UIApplicationDelegateAdaptor(PhoneAppDelegate.self) private var appDelegate

    init() {
        let store = NasStore.live()
        _store = State(initialValue: store)
        PhoneAppDelegate.store = store
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
                Tab("Verlauf", systemImage: "chart.xyaxis.line") {
                    HistoryView()
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
        // Back in the app, the records of the Mac in iCloud come first (#37)
        // and due NAS are queried at once (the planner of the core sleeps
        // while the app is suspended); leaving it asks iOS for an
        // occasional query in the background.
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .active:
                Task {
                    await store.fetchFromCloud()
                    await store.refreshAll()
                }
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
            await store.fetchFromCloud()
            await store.refreshAll()
        }
        #else
        // Menu bar app; LSUIElement keeps it out of the Dock while the main
        // window is closed. The status item, its popover and the windows
        // (main window #28, settings #21) come from `StatusItemController`.
        Settings {
            EmptyView()
        }
        .commands {
            // ⌘, in the app menu (shown while the main window is in front)
            // opens the settings window of `StatusItemController`, not this
            // empty scene.
            CommandGroup(replacing: .appSettings) {
                Button("Einstellungen …") {
                    appDelegate.showSettings()
                }
                .keyboardShortcut(",", modifiers: .command)
            }
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
}
