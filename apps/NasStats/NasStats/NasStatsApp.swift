import SwiftUI

@main
struct NasStatsApp: App {
    var body: some Scene {
        #if os(iOS)
        WindowGroup {
            ContentView()
        }
        #else
        // Menu bar only; LSUIElement keeps the app out of the Dock.
        MenuBarExtra("NAS Stats", systemImage: "externaldrive") {
            MenuBarView()
        }
        .menuBarExtraStyle(.menu)
        #endif
    }
}
