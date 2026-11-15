import SwiftUI

/// Start screen of the iOS app: the NAS with their state, worst first.
/// Provisional; the real overview follows with #17, the settings are in
/// their own tab (#21).
struct ContentView: View {
    @Environment(NasStore.self) private var store
    @State private var addingNas = false

    var body: some View {
        NavigationStack {
            Group {
                if store.entries.isEmpty {
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
                    .environment(store)
            }
        }
    }

    private var empty: some View {
        VStack(spacing: 12) {
            Image(systemName: "externaldrive")
                .font(.system(size: 56))
                .foregroundStyle(.tint)
            if let loadError = store.loadError {
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
                ForEach(store.entries) { entry in
                    HStack {
                        Circle()
                            .fill(entry.health.color)
                            .frame(width: 10, height: 10)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: entry.displayName)
                                .font(.headline)
                            Text(verbatim: "\(entry.displayModel ?? String(localized: "Modell unbekannt")) · \(entry.baseUrl)")
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(entry.health.title)
                            .foregroundStyle(.secondary)
                    }
                }
            } footer: {
                VStack(alignment: .leading, spacing: 4) {
                    if let loadError = store.loadError {
                        Text(loadError)
                            .foregroundStyle(.red)
                    }
                    coreVersionText
                }
            }
        }
    }

    private var coreVersionText: some View {
        Text(verbatim: store.demoMode ? "Demo · Core \(coreVersion())" : "Core \(coreVersion())")
            .font(.footnote.monospacedDigit())
            .foregroundStyle(.secondary)
    }
}

#Preview {
    ContentView()
        .environment(NasStore.live())
}
