import SwiftUI

/// Dialog "NAS hinzufügen": address and credentials, the certificate
/// fingerprint to confirm, then one test query. The NAS is saved only after
/// the test query succeeds; a failed one is never repeated by itself.
struct AddNasView: View {
    @Environment(NasStore.self) private var store
    #if os(macOS)
    /// Closes the window; set by `StatusItemController`.
    var onClose: () -> Void = {}
    #else
    @Environment(\.dismiss) private var dismiss
    #endif

    @State private var address = ""
    @State private var name = ""
    @State private var user = ""
    @State private var password = ""
    @State private var syncsViaICloud = false
    /// Fingerprint and the normalized address it was read from.
    @State private var certificate: (address: String, fingerprint: String)?
    @State private var busy = false
    @State private var errorText: String?
    /// Credentials the NAS refused; no new login with them (it may block
    /// the address).
    @State private var refusedCredentials: Credentials?
    /// Changes when the dialog closes; a test query started before then
    /// saves nothing.
    @State private var attempt = UUID()

    private struct Credentials: Equatable {
        let baseUrl: String
        let user: String
        let password: String
    }

    private static let vendors = ["Synology", "TrueNAS", "Unraid"]

    var body: some View {
        #if os(macOS)
        VStack(spacing: 0) {
            form
            Divider()
            HStack {
                Spacer()
                Button("Abbrechen", action: close)
                    .keyboardShortcut(.cancelAction)
                    .disabled(busy)
                Button("Sichern", action: connect)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canConnect)
            }
            .padding()
        }
        .frame(width: 480)
        .fixedSize(horizontal: false, vertical: true)
        #else
        NavigationStack {
            form
                .navigationTitle("NAS hinzufügen")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Abbrechen", action: close)
                            .disabled(busy)
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Sichern", action: connect)
                            .disabled(!canConnect)
                    }
                }
        }
        .interactiveDismissDisabled(busy)
        #endif
    }

    private var form: some View {
        Form {
            Section {
                LabeledContent("Hersteller") {
                    HStack(spacing: 6) {
                        chip("QNAP", selected: true)
                        ForEach(Self.vendors, id: \.self) { vendor in
                            chip("\(vendor) · folgt", selected: false)
                        }
                    }
                }
                TextField("Adresse", text: $address, prompt: Text(verbatim: "https://192.168.178.20:443"))
                    .font(.body.monospaced())
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    #endif
                if let addressProblem {
                    Text(addressProblem)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                TextField("Name", text: $name, prompt: Text("optional"))
                TextField("Benutzer", text: $user)
                    .autocorrectionDisabled()
                    .textContentType(.username)
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                SecureField("Passwort", text: $password)
                    .textContentType(.password)
                Toggle("Über iCloud-Schlüsselbund auf meine Geräte", isOn: $syncsViaICloud)
            } footer: {
                Text("Empfohlen: eigenes Konto mit der Rolle „System Monitoring“ (ab QTS 5.1), ohne 2-Schritt-Bestätigung. NAS Stats liest nur.")
            }
            .disabled(busy)

            Section {
                if let fingerprint = currentFingerprint {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Zertifikat bestätigen")
                            .font(.headline)
                        Text("Prüfen Sie den Fingerabdruck in QTS unter Systemsteuerung › Sicherheit › Zertifikat.")
                            .font(.footnote)
                        Text(verbatim: "SHA-256 \(fingerprint)")
                            .font(.footnote.monospaced())
                            .textSelection(.enabled)
                        Text("Danach erwartet NAS Stats genau dieses Zertifikat. Ändert es sich, stoppt die Abfrage und fragt nach.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 4)
                    Button("Zertifikat vertrauen und verbinden", action: connect)
                        .disabled(!canConnect)
                } else {
                    Button("Zertifikat abrufen", action: fetchCertificate)
                        .disabled(!canFetch)
                }
                if busy {
                    ProgressView()
                }
                if let errorText {
                    Text(errorText)
                        .font(.footnote)
                        .foregroundStyle(.red)
                } else if credentialsRefused {
                    Text("Das NAS hat diese Zugangsdaten abgelehnt. Ändern Sie Benutzer oder Passwort, um es erneut zu versuchen.")
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
            }
        }
        .formStyle(.grouped)
        .onDisappear {
            // A window may keep its state when closed; start clean.
            attempt = UUID()
            password = ""
            certificate = nil
            errorText = nil
            refusedCredentials = nil
        }
        .onChange(of: address) {
            certificate = nil
            errorText = nil
        }
    }

    private func chip(_ title: String, selected: Bool) -> some View {
        Text(title)
            .font(.footnote)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(
                Capsule().fill(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.quaternary))
            )
            .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
            .accessibilityAddTraits(selected ? .isSelected : [])
    }

    // MARK: - State

    private var normalizedAddress: String? {
        try? NasAddress.normalize(address).get()
    }

    /// Hint under the address field; nothing while the field is empty.
    private var addressProblem: String? {
        switch NasAddress.normalize(address) {
        case .success, .failure(.empty): return nil
        case let .failure(error): return connectionErrorText(error)
        }
    }

    private var trimmedUser: String {
        user.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canFetch: Bool {
        !busy && normalizedAddress != nil && !trimmedUser.isEmpty && !password.isEmpty
    }

    /// The fingerprint, as long as it belongs to the current address.
    private var currentFingerprint: String? {
        guard let certificate, certificate.address == normalizedAddress else { return nil }
        return certificate.fingerprint
    }

    private var credentialsRefused: Bool {
        guard let refusedCredentials, let baseUrl = normalizedAddress else { return false }
        return refusedCredentials
            == Credentials(baseUrl: baseUrl, user: trimmedUser, password: password)
    }

    private var canConnect: Bool {
        canFetch && currentFingerprint != nil && !credentialsRefused
    }

    // MARK: - Actions

    private func fetchCertificate() {
        guard canFetch, let baseUrl = normalizedAddress else { return }
        busy = true
        errorText = nil
        certificate = nil
        Task {
            defer { busy = false }
            do {
                let fingerprint = try await fetchFingerprint(baseUrl: baseUrl)
                certificate = (baseUrl, fingerprint)
            } catch {
                errorText = connectionErrorText(error)
            }
        }
    }

    private func connect() {
        guard canConnect, let baseUrl = normalizedAddress, let fingerprint = currentFingerprint
        else { return }
        busy = true
        errorText = nil
        let draft = NasAccount(
            id: UUID(),
            name: "",
            providerKey: NasAccount.qnapKey,
            baseUrl: baseUrl,
            fingerprint: fingerprint,
            user: trimmedUser,
            model: nil,
            syncsViaICloud: syncsViaICloud
        )
        let password = password
        let typedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let started = attempt
        Task {
            defer { busy = false }
            var account = draft
            do {
                guard let config = account.config else { return }
                let snapshot = try await store.core.testConnection(
                    config: config, user: account.user, password: password
                )
                let hostname = snapshot.nas.hostname.flatMap { $0.isEmpty ? nil : $0 }
                account.name = typedName.isEmpty
                    ? hostname ?? NasAddress.host(of: baseUrl) ?? baseUrl
                    : typedName
                account.model = snapshot.nas.model
                guard attempt == started else { return }
                // The test query counts as the first query of the NAS.
                try await store.add(
                    StoredAccount(account: account, password: password), snapshot: snapshot
                )
                guard attempt == started else { return }
                close()
            } catch {
                switch error as? FfiError {
                case .LoginFailed?, .SecondFactorRequired?:
                    refusedCredentials = Credentials(
                        baseUrl: baseUrl, user: account.user, password: password
                    )
                case .CertificateChanged?:
                    // Offers "Zertifikat abrufen" again.
                    certificate = nil
                default:
                    break
                }
                errorText = connectionErrorText(error)
            }
        }
    }

    private func close() {
        #if os(macOS)
        onClose()
        #else
        dismiss()
        #endif
    }
}

#Preview {
    AddNasView()
        .environment(NasStore.live())
}
