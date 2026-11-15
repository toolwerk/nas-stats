import SwiftUI

/// Start screen of the iOS app: the NAS with their state, worst first.
/// Provisional; the real screens follow with #17 and #21.
struct ContentView: View {
    @Environment(NasStore.self) private var store

    var body: some View {
        if store.entries.isEmpty {
            VStack(spacing: 12) {
                Image(systemName: "externaldrive")
                    .font(.system(size: 56))
                    .foregroundStyle(.tint)
                Text("NAS Stats")
                    .font(.largeTitle.bold())
                Text("Noch kein NAS verbunden")
                    .foregroundStyle(.secondary)
                footer
            }
            .padding()
        } else {
            NavigationStack {
                List {
                    Section {
                        ForEach(store.entries) { entry in
                            HStack {
                                Circle()
                                    .fill(entry.health.color)
                                    .frame(width: 10, height: 10)
                                Text(verbatim: entry.displayName)
                                Spacer()
                                Text(entry.health.title)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    } footer: {
                        footer
                    }
                }
                .navigationTitle("NAS Stats")
            }
        }
    }

    private var footer: some View {
        Text(verbatim: store.demoMode ? "Demo · Core \(coreVersion())" : "Core \(coreVersion())")
            .font(.footnote.monospacedDigit())
            .foregroundStyle(.secondary)
    }
}

#Preview {
    ContentView()
        .environment(NasStore.live())
}
