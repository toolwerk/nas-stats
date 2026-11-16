import SwiftUI

/// Settings of iPhone (tab) and Mac (window of the menu bar) (#21): the
/// NAS, iCloud Keychain (#113) and transfer (#37), the collector on the
/// NAS (#124), query interval, thresholds, notifications (#39), demo
/// mode and the privacy note.
struct SettingsView: View {
    @Environment(NasStore.self) private var store
    @State private var confirmingStopSharing = false
    /// Why the last switch of the iCloud setting did not fully work; stays
    /// until the next switch.
    @State private var sharingError: String?
    @State private var confirmingStopTransfer = false
    /// Why the last switch of the transfer did not work; stays until the
    /// next switch.
    @State private var transferError: String?
    #if os(macOS)
    /// Opens the add dialog; set by `StatusItemController`.
    var onAddNas: () -> Void = {}
    #else
    @State private var addingNas = false
    #endif
    @State private var addingCollector = false

    var body: some View {
        NavigationStack {
            form
                .navigationTitle("Einstellungen")
                .navigationDestination(for: UUID.self) { id in
                    NasSettingsView(id: id)
                }
                .navigationDestination(for: CollectorRoute.self) { _ in
                    CollectorSettingsView()
                }
        }
        .sheet(isPresented: $addingCollector) {
            AddCollectorView()
                .environment(store)
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
            collectorSection
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
                Text("Keine Telemetrie. Zustand und Verlauf bleiben auf Ihren Geräten; mit der Übertragung in iCloud gehen Zustand, Verlauf und Ereignisse über Ihre private iCloud vom Mac zum iPhone, ohne Zugänge. Zugangsdaten liegen im Schlüsselbund, geteilt nur mit „Zugänge über iCloud-Schlüsselbund teilen“. Den Sammel-Container fragt NAS Stats nur im Heimnetz ab.")
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
                Text("Die NAS und die Kopplung mit dem Sammel-Container verschwinden von Ihren anderen Geräten und bleiben nur auf diesem.")
            }
            if let cloud = store.cloud {
                transferToggle(cloud)
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
                if let cloud = store.cloud {
                    transferFooter(cloud)
                }
            }
        }
    }

    /// The transfer of state and history (#37): the Mac writes, the iPhone
    /// takes over. Turning it off on the Mac asks first, because it deletes
    /// the data in iCloud.
    private func transferToggle(_ cloud: CloudTransfer) -> some View {
        Toggle(
            cloud.role == .upload ? "Zustand und Verlauf in iCloud übertragen" : "Zustand und Verlauf vom Mac übernehmen",
            isOn: Binding(
                get: { store.transfersViaICloud },
                set: { on in
                    if !on, cloud.role == .upload {
                        confirmingStopTransfer = true
                    } else {
                        setTransfer(on)
                    }
                }
            )
        )
        .confirmationDialog(
            "Daten aus iCloud löschen?",
            isPresented: $confirmingStopTransfer,
            titleVisibility: .visible
        ) {
            Button("Löschen und ausschalten", role: .destructive) {
                setTransfer(false)
            }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            Text("Zustand, Verlauf und Ereignisse der NAS werden aus Ihrer iCloud gelöscht. Was das iPhone schon übernommen hat, bleibt dort.")
        }
    }

    @ViewBuilder
    private func transferFooter(_ cloud: CloudTransfer) -> some View {
        switch cloud.role {
        case .upload:
            Text("Überträgt Zustand, Verlauf und Ereignisse der NAS in Ihre private iCloud, ohne Zugänge.")
        case .download:
            Text("Übernimmt Zustand, Verlauf und Ereignisse der NAS, die auch auf diesem iPhone stehen, über den iCloud-Schlüsselbund geteilt. Ein NAS, das hier getrennt eingetragen ist, bekommt nichts vom Mac.")
        }
        if store.transfersViaICloud {
            switch cloud.status {
            case .transferred(let date):
                Text(cloud.role == .upload
                     ? "Zuletzt übertragen: \(StatusFormat.age(date))."
                     : "Zuletzt vom Mac: \(StatusFormat.age(date)).")
            case .none, .noData:
                if cloud.role == .download {
                    Text("Noch keine Daten vom Mac.")
                }
            case .failed(let message):
                Text(message)
                    .foregroundStyle(.red)
            }
        }
        if let transferError {
            Text(transferError)
                .foregroundStyle(.red)
        }
    }

    private func setTransfer(_ on: Bool) {
        transferError = nil
        Task {
            do {
                try await store.setTransfersViaICloud(on)
            } catch {
                transferError = connectionErrorText(error)
            }
        }
    }

    // MARK: - Collector

    /// The collector on the NAS (#124): pair, or its state with a link to
    /// its page.
    private var collectorSection: some View {
        Section {
            if store.collector.paired != nil {
                NavigationLink(value: CollectorRoute()) {
                    HStack {
                        Circle()
                            .fill(CollectorStatus.color(of: store.collector))
                            .frame(width: 10, height: 10)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Sammel-Container")
                            Text(CollectorStatus.text(of: store.collector))
                                .font(.footnote)
                                .foregroundStyle(store.collector.error == nil ? Color.secondary : Color.red)
                        }
                    }
                }
            } else {
                Button("Container koppeln …", systemImage: "plus") {
                    addingCollector = true
                }
            }
        } header: {
            Text("Sammel-Container")
        } footer: {
            Text("Holt den Verlauf aus der Zeit, in der die App nicht lief, vom Sammel-Container auf dem NAS. Nur im Heimnetz.")
        }
    }

    private func setSharing(_ shares: Bool) {
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
