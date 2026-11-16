import SwiftUI

/// Settings of iPhone (tab) and Mac (window of the menu bar) (#21): the
/// NAS, query interval, thresholds, notifications (#39), demo mode and
/// the privacy note.
struct SettingsView: View {
    @Environment(NasStore.self) private var store
    #if os(macOS)
    /// Opens the add dialog; set by `StatusItemController`.
    var onAddNas: () -> Void = {}
    #else
    @State private var addingNas = false
    #endif

    var body: some View {
        NavigationStack {
            form
                .navigationTitle("Einstellungen")
                .navigationDestination(for: UUID.self) { id in
                    NasSettingsView(id: id)
                }
        }
        #if os(macOS)
        .frame(width: 460, height: 640)
        #else
        .sheet(isPresented: $addingNas) {
            AddNasView()
                .environment(store)
        }
        #endif
    }

    private var form: some View {
        @Bindable var store = store
        return Form {
            nasSection
            Section {
                Picker("Intervall", selection: $store.queryIntervalMinutes) {
                    ForEach(NasStore.intervalChoices, id: \.self) { minutes in
                        Text("\(minutes) Min").tag(minutes)
                    }
                }
            } header: {
                Text("Abfrage")
            } footer: {
                Text("NAS Stats fragt jedes NAS frühestens nach diesem Abstand erneut ab, nie öfter als alle 5 Minuten.")
            }
            thresholdSection
            NotificationSettingsSection()
            #if os(macOS)
            MenuBarSettingsSection()
            #endif
            Section {
                Toggle("Demo-Modus", isOn: $store.demoMode)
            } header: {
                Text("Demo")
            } footer: {
                Text("Zeigt drei erfundene NAS. Die Werte sind erfunden und stammen von keinem echten Gerät.")
            }
            Section {
                Text("Keine Telemetrie. Die Daten bleiben auf dem Gerät. Zugangsdaten liegen im Schlüsselbund, über den iCloud-Schlüsselbund nur, wenn Sie das beim Hinzufügen oder später in den Einstellungen des NAS wählen.")
                    .font(.footnote)
                LabeledContent("Core") {
                    Text(verbatim: coreVersion())
                        .monospacedDigit()
                }
            } header: {
                Text("Datenschutz")
            }
        }
        .formStyle(.grouped)
    }

    // MARK: - NAS

    private var nasSection: some View {
        Section {
            ForEach(store.accounts.accounts) { account in
                NavigationLink(value: account.id) {
                    HStack {
                        Circle()
                            .fill(health(of: account).color)
                            .frame(width: 10, height: 10)
                        Text(verbatim: account.name)
                        Spacer()
                        Text(health(of: account).title)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Button("NAS hinzufügen …", systemImage: "plus") {
                #if os(macOS)
                onAddNas()
                #else
                addingNas = true
                #endif
            }
        } header: {
            Text("NAS")
        } footer: {
            if let loadError = store.loadError {
                Text(loadError)
                    .foregroundStyle(.red)
            } else if store.demoMode {
                Text("Im Demo-Modus fragt NAS Stats kein NAS ab.")
            }
        }
    }

    /// State of the NAS in the store; unknown in demo mode, where the
    /// store shows the demo NAS instead.
    private func health(of account: NasAccount) -> StoreHealth {
        store.entries.first { $0.id == account.id.uuidString }?.health ?? .unknown
    }

    // MARK: - Thresholds

    private var thresholdSection: some View {
        let fill = store.thresholds.fill
        return Section {
            Stepper(
                value: threshold(\.fill.warning),
                in: AlertThresholds.fillRange.lowerBound...(fill.critical - AlertThresholds.fillGap),
                step: 5
            ) {
                LabeledContent("Füllstand Warnung", value: percent(fill.warning))
            }
            Stepper(
                value: threshold(\.fill.critical),
                in: (fill.warning + AlertThresholds.fillGap)...AlertThresholds.fillRange.upperBound,
                step: 5
            ) {
                LabeledContent("Füllstand kritisch", value: percent(fill.critical))
            }
            Stepper(
                value: threshold(\.diskTempWarningC),
                in: AlertThresholds.diskTempRange,
                step: 1
            ) {
                LabeledContent(
                    "Plattentemperatur",
                    value: StatusFormat.temperature(store.thresholds.diskTempWarningC)
                )
            }
        } header: {
            Text("Schwellen")
        } footer: {
            Text("Ab diesen Werten zeigt NAS Stats ein Volume gelb oder rot und eine Platte gelb.")
        }
    }

    private func threshold(_ path: WritableKeyPath<AlertThresholds, Double>) -> Binding<Double> {
        Binding(
            get: { store.thresholds[keyPath: path] },
            set: { value in
                var thresholds = store.thresholds
                thresholds[keyPath: path] = value
                // The ranges of the steppers keep it valid; a stray value
                // never reaches the store.
                if thresholds.isValid {
                    store.thresholds = thresholds
                }
            }
        )
    }

    private func percent(_ value: Double) -> String {
        StatusFormat.percent(value, fractionDigits: 0)
    }
}

#Preview {
    SettingsView()
        .environment(NasStore.live())
}
