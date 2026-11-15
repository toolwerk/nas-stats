import SwiftUI

/// Start screen of the iOS app: the stored NAS as a plain list until the
/// overview arrives (#15).
struct ContentView: View {
    @Environment(NasAccounts.self) private var accounts
    @State private var addingNas = false

    var body: some View {
        NavigationStack {
            Group {
                if accounts.accounts.isEmpty {
                    empty
                } else {
                    list
                }
            }
            .navigationTitle("NAS Stats")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("NAS hinzufügen", systemImage: "plus") {
                        addingNas = true
                    }
                }
            }
            .sheet(isPresented: $addingNas) {
                AddNasView()
                    .environment(accounts)
            }
        }
    }

    private var empty: some View {
        VStack(spacing: 12) {
            Image(systemName: "externaldrive")
                .font(.system(size: 56))
                .foregroundStyle(.tint)
            if let loadError = accounts.loadError {
                Text(loadError)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
            } else {
                Text("Noch kein NAS verbunden")
                    .foregroundStyle(.secondary)
            }
            Button("NAS hinzufügen") {
                addingNas = true
            }
            .buttonStyle(.borderedProminent)
            coreVersionText
        }
        .padding()
    }

    private var list: some View {
        List {
            Section {
                ForEach(accounts.accounts) { account in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(account.name)
                            .font(.headline)
                        Text(verbatim: "\(account.model ?? String(localized: "Modell unbekannt")) · \(account.baseUrl)")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            } footer: {
                coreVersionText
            }
        }
    }

    private var coreVersionText: some View {
        Text(verbatim: "Core \(coreVersion())")
            .font(.footnote.monospacedDigit())
            .foregroundStyle(.secondary)
    }
}

#Preview {
    ContentView()
        .environment(NasAccounts())
}
