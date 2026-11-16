import SwiftUI

/// Start screen of the iOS app (#17): the state of all NAS in one sentence
/// and one card per NAS, worst first. Pull to refresh queries only NAS
/// whose interval has passed (at least 5 minutes); a tap opens the detail.
struct OverviewView: View {
    @Environment(NasStore.self) private var store
    @State private var addingNas = false

    var body: some View {
        NavigationStack {
            Group {
                if store.entries.isEmpty {
                    empty
                } else {
                    cards
                }
            }
            .navigationTitle("Übersicht")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("NAS hinzufügen", systemImage: "plus") {
                        addingNas = true
                    }
                }
            }
            .navigationDestination(for: String.self) { id in
                NasDetailView(id: id)
            }
            .sheet(isPresented: $addingNas) {
                AddNasView()
                    .environment(store)
            }
        }
    }

    private var cards: some View {
        ScrollView {
            // Head stamp and backup ages move on every minute.
            TimelineView(.everyMinute) { context in
                let now = context.date
                LazyVStack(alignment: .leading, spacing: 12) {
                    head(now: now)
                    if let loadError = store.loadError {
                        Text(loadError)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                    ForEach(store.entries) { entry in
                        NavigationLink(value: entry.id) {
                            NasCard(entry: entry, now: now, intervalMinutes: store.queryIntervalMinutes)
                        }
                        .buttonStyle(.plain)
                    }
                    coreVersionText
                        .frame(maxWidth: .infinity)
                        .padding(.top, 4)
                }
                .padding(.horizontal)
                .padding(.bottom)
            }
        }
        .background(Color.overviewBackground)
        .refreshable {
            await store.refreshAll()
        }
    }

    private func head(now: Date) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: OverviewSummary.sentence(store.entries, demo: store.demoMode))
                .font(.subheadline)
            Text(verbatim: OverviewSummary.stamp(store.entries, now: now))
                .font(.footnote)
                .foregroundStyle(.secondary)
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

    private var coreVersionText: some View {
        Text(verbatim: store.demoMode ? "Demo · Core \(coreVersion())" : "Core \(coreVersion())")
            .font(.footnote.monospacedDigit())
            .foregroundStyle(.secondary)
    }
}

// MARK: - Previews

/// Keychain stand-in without entries, so the previews never read real NAS.
private struct PreviewAccountStore: AccountStore {
    func loadAll() throws -> [StoredAccount] { [] }
    func save(_ stored: StoredAccount) throws {}
    func delete(id: UUID) throws {}
}

/// Store on its own defaults suite, without queries.
@MainActor
private func previewStore(demo: Bool) -> NasStore {
    let defaults = UserDefaults(suiteName: "nasstats-preview-overview") ?? .standard
    defaults.set(demo, forKey: NasStore.demoModeKey)
    return NasStore(
        accounts: NasAccounts(keychain: PreviewAccountStore()),
        defaults: defaults,
        demoSource: { demoSnapshots(now: $0, thresholds: $1) }
    )
}

#Preview("Übersicht Demo hell") {
    OverviewView()
        .environment(previewStore(demo: true))
        .environment(\.locale, Locale(identifier: "de_DE"))
        .preferredColorScheme(.light)
}

#Preview("Übersicht Demo dunkel") {
    OverviewView()
        .environment(previewStore(demo: true))
        .environment(\.locale, Locale(identifier: "de_DE"))
        .preferredColorScheme(.dark)
}

#Preview("Übersicht leer") {
    OverviewView()
        .environment(previewStore(demo: false))
}
