import SwiftUI

/// Dialog "Container koppeln" (#124): address and key of the collector on
/// the NAS, the certificate fingerprint to confirm, then one test request.
/// The pairing is saved only when the collector answers with this key.
struct AddCollectorView: View {
    /// Address to start with, e.g. for "Neu koppeln".
    var initialAddress = ""

    @Environment(NasStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    @State private var address = ""
    @State private var token = ""
    /// Fingerprint and the normalized address it was read from.
    @State private var certificate: (address: String, fingerprint: String)?
    @State private var busy = false
    @State private var errorText: String?

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
                Button("Koppeln", action: pair)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canPair)
            }
            .padding()
        }
        .frame(width: 480)
        .fixedSize(horizontal: false, vertical: true)
        #else
        NavigationStack {
            form
                .navigationTitle("Container koppeln")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Abbrechen") { dismiss() }
                            .disabled(busy)
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Koppeln", action: pair)
                            .disabled(!canPair)
                    }
                }
        }
        .interactiveDismissDisabled(busy)
        #endif
    }

    private var form: some View {
        Form {
            Section {
                TextField("Adresse", text: $address, prompt: Text(verbatim: "https://192.168.178.20:8743"))
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
                SecureField("Schlüssel", text: $token)
            } footer: {
                Text("Adresse und Schlüssel zeigt `nas-collector pairing` im Container. Ohne Port gilt 8743.")
            }
            .disabled(busy)

            Section {
                if let fingerprint = currentFingerprint {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Zertifikat bestätigen")
                            .font(.headline)
                        Text("Vergleichen Sie den Fingerabdruck mit der Ausgabe von `nas-collector pairing`.")
                            .font(.footnote)
                        Text(verbatim: "SHA-256 \(fingerprint)")
                            .font(.footnote.monospaced())
                            .textSelection(.enabled)
                    }
                    .padding(.vertical, 4)
                    Button("Zertifikat vertrauen und koppeln", action: pair)
                        .disabled(!canPair)
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
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            if address.isEmpty {
                address = initialAddress
            }
        }
        .onChange(of: address) {
            certificate = nil
            errorText = nil
        }
    }

    // MARK: - State

    private var normalizedAddress: String? {
        try? CollectorAddress.normalize(address).get()
    }

    /// Hint under the address field; nothing while the field is empty.
    private var addressProblem: String? {
        switch CollectorAddress.normalize(address) {
        case .success, .failure(.empty): return nil
        case let .failure(error): return connectionErrorText(error)
        }
    }

    private var trimmedToken: String {
        token.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canFetch: Bool {
        !busy && normalizedAddress != nil
    }

    /// The fingerprint, as long as it belongs to the current address.
    private var currentFingerprint: String? {
        guard let certificate, certificate.address == normalizedAddress else { return nil }
        return certificate.fingerprint
    }

    private var canPair: Bool {
        canFetch && currentFingerprint != nil && !trimmedToken.isEmpty
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
                errorText = collectorErrorText(error)
            }
        }
    }

    private func pair() {
        guard canPair, let baseUrl = normalizedAddress, let fingerprint = currentFingerprint
        else { return }
        busy = true
        errorText = nil
        let token = trimmedToken
        Task {
            defer { busy = false }
            do {
                try await store.pairCollector(baseUrl: baseUrl, fingerprint: fingerprint, token: token)
                dismiss()
            } catch {
                if case .CertificateChanged? = error as? FfiError {
                    // Offers "Zertifikat abrufen" again.
                    certificate = nil
                }
                errorText = collectorErrorText(error)
            }
        }
    }
}

#Preview {
    AddCollectorView()
        .environment(NasStore.live())
}
