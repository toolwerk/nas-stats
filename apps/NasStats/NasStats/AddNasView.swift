import SwiftUI

/// Dialog "NAS hinzufügen": address and credentials, the certificate
/// fingerprint to confirm, then one test query. The NAS is saved only after
/// the test query succeeds; a failed one is never repeated by itself.
struct AddNasView: View {
    @Environment(NasStore.self) private var store
    #if os(macOS)
    /// Closes the window; set by `StatusItemController`.
    var onClose: () -> Void = {}
    /// False once the window is closed; a test query still running then
    /// saves nothing (the view state may be gone with the window).
    var isOpen: () -> Bool = { true }
    #else
    @Environment(\.dismiss) private var dismiss
    #endif

    @State private var vendor: ProviderKind = .qnap
    @State private var address = ""
    @State private var name = ""
    @State private var user = ""
    @State private var password = ""
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
        let vendor: ProviderKind
        let baseUrl: String
        let user: String
        let password: String
    }

    private static let vendors: [ProviderKind] = [.qnap, .unraid, .synology, .trueNas]

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
                        ForEach(Self.vendors, id: \.self) { kind in
                            if kind.isSelectable {
                                Button { vendor = kind } label: {
                                    chip(kind.displayName, selected: vendor == kind)
                                }
                                .buttonStyle(.plain)
                                .disabled(busy)
                            } else {
                                chip("\(kind.displayName) · folgt", selected: false)
                            }
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
                if vendor.usesUser {
                    TextField("Benutzer", text: $user)
                        .autocorrectionDisabled()
                        .textContentType(.username)
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                }
                SecureField(vendor.secretLabel, text: $password)
                    .textContentType(.password)
            } footer: {
                Text(vendor.accessHint)
            }
            .disabled(busy)

            Section {
                if let fingerprint = currentFingerprint {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Zertifikat bestätigen")
                            .font(.headline)
                        Text(vendor.certificateHint)
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
                    Text(vendor.refusedHint)
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
        .onChange(of: vendor) {
            // The secret of one vendor is no secret of another; the
            // certificate belongs to the address, not to the vendor. A
            // refusal stays: it names its vendor.
            password = ""
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

    /// Empty for a vendor with an API key instead of a user.
    private var trimmedUser: String {
        vendor.usesUser ? user.trimmingCharacters(in: .whitespacesAndNewlines) : ""
    }

    /// The password as typed; an API key without the blanks a paste adds.
    private var secret: String {
        vendor.usesUser ? password : password.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canFetch: Bool {
        !busy && normalizedAddress != nil && (!vendor.usesUser || !trimmedUser.isEmpty)
            && !secret.isEmpty
    }

    /// The fingerprint, as long as it belongs to the current address.
    private var currentFingerprint: String? {
        guard let certificate, certificate.address == normalizedAddress else { return nil }
        return certificate.fingerprint
    }

    private var credentialsRefused: Bool {
        guard let refusedCredentials, let baseUrl = normalizedAddress else { return false }
        return refusedCredentials
            == Credentials(vendor: vendor, baseUrl: baseUrl, user: trimmedUser, password: secret)
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
                errorText = connectionErrorText(error, provider: vendor)
            }
        }
    }

    private func connect() {
        guard canConnect, let baseUrl = normalizedAddress, let fingerprint = currentFingerprint
        else { return }
        guard let providerKey = vendor.accountKey else { return }
        busy = true
        errorText = nil
        let vendor = vendor
        let draft = NasAccount(
            id: UUID(),
            name: "",
            providerKey: providerKey,
            baseUrl: baseUrl,
            fingerprint: fingerprint,
            user: trimmedUser,
            model: nil,
            // The store sets it from the setting of the settings (#113).
            syncsViaICloud: false
        )
        let password = secret
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
                guard attempt == started, stillOpen else { return }
                // The test query counts as the first query of the NAS.
                try await store.add(
                    StoredAccount(account: account, password: password), snapshot: snapshot
                )
                guard attempt == started, stillOpen else { return }
                close()
            } catch {
                switch error as? FfiError {
                case .LoginFailed?, .SecondFactorRequired?, .IpBlocked?, .PermissionDenied?:
                    refusedCredentials = Credentials(
                        vendor: vendor, baseUrl: baseUrl, user: account.user, password: password
                    )
                case .CertificateChanged?:
                    // Offers "Zertifikat abrufen" again.
                    certificate = nil
                default:
                    break
                }
                errorText = connectionErrorText(error, provider: vendor)
            }
        }
    }

    /// The dialog is still shown; always true on iOS.
    private var stillOpen: Bool {
        #if os(macOS)
        isOpen()
        #else
        true
        #endif
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
