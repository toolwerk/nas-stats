import SwiftUI

/// Settings of iPhone (tab) and Mac (window of the menu bar) (#21): the
/// NAS, iCloud Keychain (#113), query interval, thresholds, demo mode and
/// the privacy note.
struct SettingsView: View {
    @Environment(NasStore.self) private var store
    @State private var confirmingStopSharing = false
    @State private var switchingSharing = false
    /// Why the last switch of the iCloud setting did not fully work; stays
    /// until the next switch.
    @State private var sharingError: String?
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
            iCloudSection
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
                Text("Keine Telemetrie. Zustand und Verlauf bleiben auf dem Gerät. Zugangsdaten liegen im Schlüsselbund, geteilt nur mit „Zugänge über iCloud-Schlüsselbund teilen“.")
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

    // MARK: - iCloud

    /// One switch for the access of every NAS (#113). Turning it off asks
    /// first, because the NAS go from the other devices.
    private var iCloudSection: some View {
        Section {
            Toggle("Zugänge über iCloud-Schlüsselbund teilen", isOn: Binding(
                get: { store.sharesAccessViaICloud },
                set: { shares in
                    if shares {
                        setSharing(true)
                    } else {
                        confirmingStopSharing = true
                    }
                }
            ))
            .disabled(switchingSharing)
            .confirmationDialog(
                "Zugänge nicht mehr teilen?",
                isPresented: $confirmingStopSharing,
                titleVisibility: .visible
            ) {
                Button("Nicht mehr teilen", role: .destructive) {
                    setSharing(false)
                }
                Button("Abbrechen", role: .cancel) {}
            } message: {
                Text("Die NAS verschwinden von Ihren anderen Geräten und bleiben nur auf diesem.")
            }
        } header: {
            Text("iCloud")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text("Gilt für alle NAS auf diesem Gerät: Adresse, Benutzer und Passwort erscheinen auf Ihren Geräten mit demselben Apple-Account. Ohne iCloud-Schlüsselbund bleiben sie hier, bis Sie ihn einschalten. Zustand und Verlauf überträgt dieser Schalter nicht.")
                if let sharingError {
                    Text(sharingError)
                        .foregroundStyle(.red)
                }
            }
        }
    }

    private func setSharing(_ shares: Bool) {
        switchingSharing = true
        defer { switchingSharing = false }
        do {
            let failed = try store.setSharesAccessViaICloud(shares)
            sharingError = failed > 0 ? String(localized: "Nicht umgestellt: \(failed) NAS.") : nil
        } catch {
            sharingError = connectionErrorText(error)
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
