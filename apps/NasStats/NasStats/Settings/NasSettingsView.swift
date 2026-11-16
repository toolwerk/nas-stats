import SwiftUI

/// Settings of one NAS (#21): name, access, start again after a block,
/// trust a new certificate (#76) and remove.
struct NasSettingsView: View {
    let id: UUID

    @Environment(NasStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    @State private var name = ""
    @State private var busy = false
    @State private var errorText: String?
    /// Fingerprint read by "Zertifikat prüfen".
    @State private var fetchedFingerprint: String?
    @State private var changingAccess = false
    @State private var confirmingRemove = false

    var body: some View {
        content
            .navigationTitle(account?.name ?? "")
            .onAppear { name = account?.name ?? "" }
            .onDisappear(perform: saveName)
            .sheet(isPresented: $changingAccess) {
                if let account {
                    ChangeAccessView(account: account)
                        .environment(store)
                }
            }
    }

    @ViewBuilder
    private var content: some View {
        #if os(macOS)
        // The settings window has no toolbar for the system back button (#99).
        VStack(spacing: 0) {
            macHeader
            Divider()
            page(account)
        }
        #else
        page(account)
        #endif
    }

    @ViewBuilder
    private func page(_ account: NasAccount?) -> some View {
        if let account {
            form(account)
        } else {
            Text("Dieses NAS ist nicht mehr gespeichert.")
                .foregroundStyle(.secondary)
        }
    }

    #if os(macOS)
    /// Back button to the list; stays usable while an action runs, like the
    /// system back on the iPhone. Dismissing saves the name (`onDisappear`).
    private var macHeader: some View {
        HStack {
            Button {
                dismiss()
            } label: {
                Label("Einstellungen", systemImage: "chevron.left")
            }
            .keyboardShortcut("[", modifiers: .command)
            Text(verbatim: account?.name ?? "")
                .font(.headline)
            Spacer()
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }
    #endif

    private var account: NasAccount? {
        store.accounts.accounts.first { $0.id == id }
    }

    /// The entry of the store; `nil` in demo mode.
    private var entry: NasEntry? {
        store.entries.first { $0.id == id.uuidString }
    }

    private func form(_ account: NasAccount) -> some View {
        Form {
            Section {
                // Locked while an action runs: it stores the account with
                // the name it started with.
                TextField("Name", text: $name)
                    .onSubmit(saveName)
                    .disabled(busy)
                LabeledContent("Adresse") {
                    Text(verbatim: account.baseUrl)
                        .textSelection(.enabled)
                }
                LabeledContent("Modell") {
                    Text(verbatim: entry?.displayModel ?? account.model ?? String(localized: "unbekannt"))
                }
                LabeledContent("Benutzer") {
                    Text(verbatim: account.user)
                }
            }
            if let entry, entry.blocked || entry.localError != nil {
                blockedSection(entry)
            }
            if let entry, entry.blocked || entry.problem?.kind == .certificateChanged {
                certificateSection(account)
            }
            Section {
                Button("Zugang ändern …") {
                    changingAccess = true
                }
                .disabled(busy)
                Button("NAS entfernen", role: .destructive) {
                    confirmingRemove = true
                }
                .disabled(busy)
                .confirmationDialog(
                    "„\(account.name)“ entfernen?",
                    isPresented: $confirmingRemove,
                    titleVisibility: .visible
                ) {
                    Button("NAS entfernen", role: .destructive, action: remove)
                } message: {
                    Text(account.syncsViaICloud
                        ? "Name und Zugang werden auf allen Ihren Geräten gelöscht."
                        : "Name und Zugang werden auf diesem Gerät gelöscht.")
                }
            } footer: {
                VStack(alignment: .leading, spacing: 4) {
                    if busy {
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

    private func blockedSection(_ entry: NasEntry) -> some View {
        Section {
            if let reason = entry.localError ?? entry.problem?.message {
                Text(reason)
                    .font(.footnote)
            }
            Button("Erneut versuchen", action: resume)
                .disabled(busy)
        } header: {
            Text("Angehalten")
        } footer: {
            Text("Versucht es genau einmal mit dem gespeicherten Zugang.")
        }
    }

    private func certificateSection(_ account: NasAccount) -> some View {
        Section {
            Button("Zertifikat prüfen", action: fetchCertificate)
                .disabled(busy)
            if let fetchedFingerprint {
                if Self.same(fetchedFingerprint, account.fingerprint) {
                    Text("Zertifikat unverändert")
                        .font(.footnote)
                } else {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(verbatim: "alt SHA-256 \(account.fingerprint)")
                        Text(verbatim: "neu SHA-256 \(fetchedFingerprint)")
                    }
                    .font(.footnote.monospaced())
                    .textSelection(.enabled)
                    Button("Neuem Zertifikat vertrauen") {
                        confirm(fetchedFingerprint)
                    }
                    .disabled(busy)
                }
            }
        } header: {
            Text("Zertifikat")
        } footer: {
            Text("Prüfen Sie den neuen Fingerabdruck in QTS unter Systemsteuerung › Sicherheit › Zertifikat, bevor Sie ihm vertrauen.")
        }
    }

    private static func same(_ lhs: String, _ rhs: String) -> Bool {
        lhs.caseInsensitiveCompare(rhs) == .orderedSame
    }

    // MARK: - Actions

    private func saveName() {
        // A running action stores the account with the name it started
        // with; renaming now would let keychain and list disagree.
        guard !busy, let account, name.trimmingCharacters(in: .whitespacesAndNewlines) != account.name else {
            return
        }
        do {
            try store.rename(id: id, to: name)
            errorText = nil
        } catch {
            errorText = connectionErrorText(error)
            name = account.name
        }
    }

    private func resume() {
        run { try await store.resume(id: id) }
    }

    private func fetchCertificate() {
        guard let account else { return }
        fetchedFingerprint = nil
        run { fetchedFingerprint = try await fetchFingerprint(baseUrl: account.baseUrl) }
    }

    private func confirm(_ fingerprint: String) {
        run {
            try await store.confirmFingerprint(id: id, fingerprint: fingerprint)
            fetchedFingerprint = nil
        }
    }

    private func remove() {
        run {
            try await store.remove(id: id)
            dismiss()
        }
    }

    /// Runs one action of the user; the buttons stay locked meanwhile.
    private func run(_ action: @escaping () async throws -> Void) {
        guard !busy else { return }
        busy = true
        errorText = nil
        Task {
            defer { busy = false }
            do {
                try await action()
            } catch {
                errorText = connectionErrorText(error)
            }
        }
    }
}

/// Sheet "Zugang ändern": new user and password for a stored NAS. A
/// combination the NAS refused is not sent again (it may block the address).
private struct ChangeAccessView: View {
    let account: NasAccount

    @Environment(NasStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    @State private var user = ""
    @State private var password = ""
    @State private var busy = false
    @State private var errorText: String?
    @State private var refused: Credentials?

    private struct Credentials: Equatable {
        let user: String
        let password: String
    }

    var body: some View {
        #if os(macOS)
        VStack(spacing: 0) {
            form
            Divider()
            HStack {
                Spacer()
                Button("Abbrechen") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .disabled(busy)
                Button("Sichern", action: save)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSave)
            }
            .padding()
        }
        .frame(width: 420)
        .fixedSize(horizontal: false, vertical: true)
        #else
        NavigationStack {
            form
                .navigationTitle("Zugang ändern")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Abbrechen") { dismiss() }
                            .disabled(busy)
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Sichern", action: save)
                            .disabled(!canSave)
                    }
                }
        }
        .interactiveDismissDisabled(busy)
        #endif
    }

    private var form: some View {
        Form {
            Section {
                TextField("Benutzer", text: $user)
                    .autocorrectionDisabled()
                    .textContentType(.username)
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                SecureField("Passwort", text: $password)
                    .textContentType(.password)
            } footer: {
                if let errorText {
                    Text(errorText)
                        .foregroundStyle(.red)
                } else if isRefused {
                    Text("Das NAS hat diese Zugangsdaten abgelehnt. Ändern Sie Benutzer oder Passwort, um es erneut zu versuchen.")
                        .foregroundStyle(.red)
                }
            }
            .disabled(busy)
            if busy {
                ProgressView()
            }
        }
        .formStyle(.grouped)
        .onAppear { user = account.user }
    }

    private var trimmedUser: String {
        user.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var isRefused: Bool {
        refused == Credentials(user: trimmedUser, password: password)
    }

    private var canSave: Bool {
        !busy && !trimmedUser.isEmpty && !password.isEmpty && !isRefused
    }

    private func save() {
        guard canSave else { return }
        let credentials = Credentials(user: trimmedUser, password: password)
        busy = true
        errorText = nil
        Task {
            defer { busy = false }
            do {
                try await store.updateCredentials(
                    id: account.id, user: credentials.user, password: credentials.password
                )
            } catch {
                errorText = connectionErrorText(error)
                return
            }
            // The core logs in with the query after the change; a refused
            // login blocks the NAS again.
            let entry = store.entries.first { $0.id == account.id.uuidString }
            if let entry, entry.blocked {
                switch entry.problem?.kind {
                case .loginFailed?, .secondFactorRequired?:
                    refused = credentials
                default:
                    errorText = entry.localError ?? entry.problem?.message
                        ?? String(localized: "Das NAS bleibt angehalten.")
                }
                return
            }
            dismiss()
        }
    }
}
