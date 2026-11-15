#if os(macOS)
import SwiftUI

/// Menu of the menu bar icon.
struct MenuBarView: View {
    @Environment(NasAccounts.self) private var accounts
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text("NAS Stats")
        if let loadError = accounts.loadError {
            Text(loadError)
        }
        if accounts.accounts.isEmpty {
            if accounts.loadError == nil {
                Text("Noch kein NAS verbunden")
            }
        } else {
            ForEach(accounts.accounts) { account in
                Text(account.name)
            }
        }
        Divider()
        Button("NAS hinzufügen …") {
            openWindow(id: "add-nas")
            NSApplication.shared.activate()
        }
        Divider()
        Text(verbatim: "Core \(coreVersion())")
        Button("Beenden") {
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
#endif
