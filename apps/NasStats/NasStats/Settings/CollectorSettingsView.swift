import SwiftUI

/// Value of the navigation link to `CollectorSettingsView`.
struct CollectorRoute: Hashable {}

/// Settings of the paired collector (#124): address, certificate, state of
/// the sync, which NAS of the collector go into which NAS of the app, sync
/// now, pair again and remove.
struct CollectorSettingsView: View {
    @Environment(NasStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    @State private var busy = false
    @State private var errorText: String?
    @State private var pairingAgain = false
    @State private var confirmingRemove = false

    private var collector: CollectorSync { store.collector }

    var body: some View {
        content
            .navigationTitle("Sammel-Container")
            .sheet(isPresented: $pairingAgain) {
                AddCollectorView(initialAddress: collector.paired?.baseUrl ?? "")
                    .environment(store)
            }
    }

    @ViewBuilder
    private var content: some View {
        #if os(macOS)
        // The settings window has no toolbar for the system back button (#99).
        VStack(spacing: 0) {
            macHeader
            Divider()
            page
        }
        #else
        page
        #endif
    }

    @ViewBuilder
    private var page: some View {
        if let paired = collector.paired {
            form(paired)
        } else {
            Text("Kein Sammel-Container gekoppelt.")
                .foregroundStyle(.secondary)
        }
    }

    #if os(macOS)
    private var macHeader: some View {
        HStack {
            Button {
                dismiss()
            } label: {
                Label("Einstellungen", systemImage: "chevron.left")
            }
            .keyboardShortcut("[", modifiers: .command)
            Text("Sammel-Container")
                .font(.headline)
            Spacer()
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }
    #endif

    private func form(_ paired: StoredCollector) -> some View {
        Form {
            Section {
                LabeledContent("Adresse") {
                    Text(verbatim: paired.baseUrl)
                        .textSelection(.enabled)
                }
                LabeledContent("Zertifikat") {
                    Text(verbatim: "SHA-256 \(paired.fingerprint)")
                        .font(.footnote.monospaced())
                        .textSelection(.enabled)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                LabeledContent("Abgleich") {
                    HStack(spacing: 6) {
                        Circle()
                            .fill(CollectorStatus.color(of: collector))
                            .frame(width: 10, height: 10)
                        Text(CollectorStatus.text(of: collector))
                            .foregroundStyle(collector.error == nil ? Color.secondary : Color.red)
                    }
                }
            }
            nasSection
            Section {
                Button("Jetzt abgleichen", action: syncNow)
                    .disabled(busy || collector.running)
                Button("Neu koppeln …") {
                    pairingAgain = true
                }
                .disabled(busy)
                Button("Container entfernen", role: .destructive) {
                    confirmingRemove = true
                }
                .disabled(busy)
                .confirmationDialog(
                    "Sammel-Container entfernen?",
                    isPresented: $confirmingRemove,
                    titleVisibility: .visible
                ) {
                    Button("Container entfernen", role: .destructive, action: remove)
                } message: {
                    Text(paired.syncsViaICloud
                        ? "Die Kopplung wird auf allen Ihren Geräten gelöscht. Der übernommene Verlauf bleibt."
                        : "Die Kopplung wird auf diesem Gerät gelöscht. Der übernommene Verlauf bleibt.")
                }
            } footer: {
                VStack(alignment: .leading, spacing: 4) {
                    if busy || collector.running {
                        ProgressView()
                    }
                    if let errorText {
                        Text(errorText)
                            .foregroundStyle(.red)
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    // MARK: - NAS of the collector

    private var nasSection: some View {
        let assignments = collector.assignments
        return Section {
            if collector.nas.isEmpty {
                Text(collector.listed
                    ? "Der Container fragt kein NAS ab."
                    : "Noch nicht abgeglichen.")
                    .foregroundStyle(.secondary)
            }
            ForEach(collector.nas, id: \.id) { nas in
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: Self.title(of: nas))
                    Picker("Übernehmen in", selection: binding(for: nas.id)) {
                        Text("Nicht übernehmen").tag(UUID?.none)
                        ForEach(choices(for: nas.id, assignments: assignments)) { account in
                            Text(verbatim: account.name).tag(UUID?.some(account.id))
                        }
                    }
                    .disabled(busy)
                    if assignments[nas.id]?.automatic == true {
                        Text("automatisch über das Zertifikat")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    if let problem = collector.nasErrors[nas.id] ?? nas.error
                        ?? (nas.blocked ? String(localized: "Der Container hat die Abfrage angehalten.") : nil) {
                        Text(problem)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                }
                .padding(.vertical, 2)
            }
        } header: {
            Text("NAS im Container")
        } footer: {
            Text("Werte eines NAS im Container kommen in den Verlauf des NAS, in das sie übernommen werden.")
        }
    }

    /// "nas-keller · TS-464", else the address.
    private static func title(of nas: CollectorNas) -> String {
        let name = nas.name.flatMap { $0.isEmpty ? nil : $0 }
            ?? NasAddress.host(of: nas.baseUrl) ?? nas.baseUrl
        guard let model = nas.model, !model.isEmpty else { return name }
        return "\(name) · \(model)"
    }

    /// NAS of the app that no other NAS of the collector goes into, and
    /// the one this NAS goes into.
    private func choices(
        for collectorNasId: String, assignments: [String: CollectorSync.Assignment]
    ) -> [NasAccount] {
        let taken = Set(assignments.filter { $0.key != collectorNasId }.map(\.value.appNasId))
        return store.accounts.accounts.filter { !taken.contains($0.id) }
    }

    private func binding(for collectorNasId: String) -> Binding<UUID?> {
        Binding(
            get: { collector.assignments[collectorNasId]?.appNasId },
            set: { appNasId in
                do {
                    try store.assignCollectorNas(collectorNasId, to: appNasId)
                    errorText = nil
                } catch {
                    errorText = connectionErrorText(error)
                }
            }
        )
    }

    // MARK: - Actions

    private func syncNow() {
        guard !busy else { return }
        busy = true
        errorText = nil
        Task {
            defer { busy = false }
            await collector.sync()
        }
    }

    private func remove() {
        do {
            try collector.remove()
            dismiss()
        } catch {
            errorText = connectionErrorText(error)
        }
    }
}

/// State of the sync for the settings.
@MainActor
enum CollectorStatus {
    static func color(of collector: CollectorSync) -> Color {
        if collector.error != nil { return .red }
        if !collector.nasErrors.isEmpty { return .orange }
        return collector.state.lastSuccessAt == nil ? .gray : .green
    }

    static func text(of collector: CollectorSync) -> String {
        if let error = collector.error { return error }
        if collector.running { return String(localized: "gleicht ab …") }
        guard let last = collector.state.lastSuccessAt else {
            return String(localized: "noch nicht abgeglichen")
        }
        let text = collectorLastSyncText(Date(timeIntervalSince1970: TimeInterval(last)))
        let failed = collector.nasErrors.count
        return failed == 0 ? text : String(localized: "\(text), \(failed) NAS mit Fehler")
    }
}

#Preview {
    NavigationStack {
        CollectorSettingsView()
    }
    .environment(NasStore.live())
}
