#if os(macOS)
import SwiftUI

/// Menu of the menu bar icon.
struct MenuBarView: View {
    var body: some View {
        Text("NAS Stats")
        Text(verbatim: "Core \(coreVersion())")
        Divider()
        Button("Beenden") {
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
#endif
