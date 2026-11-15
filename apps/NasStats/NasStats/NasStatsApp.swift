import SwiftUI

@main
struct NasStatsApp: App {
    /// Stored NAS, loaded from the keychain and registered with the core.
    @State private var accounts = NasAccounts()

    var body: some Scene {
        #if os(iOS)
        WindowGroup {
            ContentView()
                .environment(accounts)
        }
        #else
        // Menu bar only; LSUIElement keeps the app out of the Dock.
        MenuBarExtra("NAS Stats", systemImage: "externaldrive") {
            MenuBarView()
                .environment(accounts)
        }
        .menuBarExtraStyle(.menu)

        Window("NAS hinzufügen", id: "add-nas") {
            AddNasView()
                .environment(accounts)
        }
        .windowResizability(.contentSize)
        #endif
    }
}
