#if os(macOS)
import SwiftUI

/// Mac part of the settings (#21): display of the menu bar symbol and the
/// login item (#27).
struct MenuBarSettingsSection: View {
    @AppStorage(MenuBarDisplay.defaultsKey) private var display: MenuBarDisplay = .symbol
    /// Mirrors the system; read again when a window of the app becomes key
    /// or a menu closes, since the right-click menu may have changed it.
    @State private var launchAtLogin = LaunchAtLogin.isEnabled

    var body: some View {
        Section {
            Picker("Anzeige", selection: $display) {
                ForEach(MenuBarDisplay.allCases, id: \.self) { choice in
                    Text(choice.title).tag(choice)
                }
            }
            Toggle("Bei Anmeldung starten", isOn: Binding(
                get: { launchAtLogin },
                set: { enabled in
                    LaunchAtLogin.setEnabled(enabled)
                    launchAtLogin = LaunchAtLogin.isEnabled
                }
            ))
        } header: {
            Text("Menüleiste")
        }
        .onAppear { launchAtLogin = LaunchAtLogin.isEnabled }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
            launchAtLogin = LaunchAtLogin.isEnabled
        }
        .onReceive(NotificationCenter.default.publisher(for: NSMenu.didEndTrackingNotification)) { _ in
            launchAtLogin = LaunchAtLogin.isEnabled
        }
    }
}
#endif
