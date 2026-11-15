#if os(macOS)
import SwiftUI

/// Settings of the Mac app, provisional until #21: display of the menu
/// bar, login item, demo mode and the core version.
struct MacSettingsView: View {
    @Environment(NasStore.self) private var store
    @AppStorage(MenuBarDisplay.defaultsKey) private var display: MenuBarDisplay = .symbol
    /// Mirrors the system; read again when the window appears.
    @State private var launchAtLogin = LaunchAtLogin.isEnabled

    var body: some View {
        @Bindable var store = store
        Form {
            Picker("Anzeige", selection: $display) {
                ForEach(MenuBarDisplay.allCases, id: \.self) { choice in
                    Text(choice.title).tag(choice)
                }
            }
            Toggle("Bei Anmeldung starten", isOn: Binding(
                get: { launchAtLogin },
                set: { _ in
                    LaunchAtLogin.toggle()
                    launchAtLogin = LaunchAtLogin.isEnabled
                }
            ))
            Toggle("Demo-Modus", isOn: $store.demoMode)
            LabeledContent("Core") {
                Text(verbatim: coreVersion())
                    .monospacedDigit()
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { launchAtLogin = LaunchAtLogin.isEnabled }
    }
}
#endif
