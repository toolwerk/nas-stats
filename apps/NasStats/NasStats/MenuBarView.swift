#if os(macOS)
import SwiftUI

/// Menu of the menu bar icon: the NAS with their state, worst first.
/// Provisional; the real menu bar follows with #27 and #28.
struct MenuBarView: View {
    @Environment(NasStore.self) private var store
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        @Bindable var store = store
        Text("NAS Stats")
        if let loadError = store.loadError {
            Text(loadError)
        }
        if store.entries.isEmpty {
            if store.loadError == nil {
                Text("Noch kein NAS verbunden")
            }
        } else {
            ForEach(store.entries) { entry in
                Label {
                    Text(verbatim: "\(entry.displayName) – \(entry.health.title)")
                } icon: {
                    Image(systemName: entry.health.symbol)
                }
            }
        }
        Divider()
        Button("NAS hinzufügen …") {
            openWindow(id: "add-nas")
            NSApplication.shared.activate()
        }
        Toggle("Demo-Modus", isOn: $store.demoMode)
        Divider()
        Text(verbatim: "Core \(coreVersion())")
        Button("Beenden") {
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
#endif
