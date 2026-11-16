import SwiftUI

/// Tab „Ereignisse“ (#20): the problems of all NAS of the last 30 days as a
/// chronicle, grouped by day, newest first. The filter is not saved.
struct EventsView: View {
    /// How far back the list reaches; the core never deletes old events.
    private static let windowDays = 30

    @Environment(NasStore.self) private var store
    @State private var filter = EventFilter()
    @State private var events: [NasEvent] = []
    @State private var loadFailed = false

    var body: some View {
        NavigationStack {
            let content = EventsContent(
                events: events,
                names: Dictionary(store.entries.map { ($0.id, $0.displayName) }) { first, _ in first },
                filter: filter,
                now: Date()
            )
            List {
                Section {
                    Picker("Anzeige", selection: $filter.onlyOpen) {
                        Text("Alle").tag(false)
                        Text("Offen").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .listRowBackground(Color.clear)
                    if store.demoMode {
                        Text("Demo-Ereignisse")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    if loadFailed {
                        Text("Ereignisse nicht lesbar")
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                }
                if content.days.isEmpty {
                    Section {
                        Text(content.hasAnyEvents ? "Keine Ereignisse für diesen Filter" : "Noch keine Ereignisse")
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .center)
                    }
                }
                ForEach(content.days) { day in
                    Section(day.title) {
                        ForEach(day.rows) { row in
                            EventRowView(row: row)
                        }
                    }
                }
            }
            .navigationTitle("Ereignisse")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    filterMenu
                }
            }
            .refreshable {
                await store.refreshAll()
                load()
            }
        }
        // Loads again after every new state of a NAS (a query, a removal)
        // and when the demo mode is switched.
        .task(id: reloadKey) {
            load()
        }
    }

    private var reloadKey: [String] {
        [store.demoMode.description]
            + store.entries.map { "\($0.id)|\($0.snapshot?.observedAt ?? 0)|\($0.lastAttemptAt ?? 0)" }
    }

    private var filterMenu: some View {
        Menu("Filter", systemImage: "line.3.horizontal.decrease.circle") {
            Picker("NAS", selection: $filter.nasId) {
                Text("Alle NAS").tag(String?.none)
                ForEach(store.entries) { entry in
                    Text(verbatim: entry.displayName).tag(Optional(entry.id))
                }
            }
            Picker("Art", selection: $filter.category) {
                Text("Alle Arten").tag(NotificationCategory?.none)
                ForEach(NotificationCategory.allCases, id: \.self) { category in
                    Text(category.title).tag(Optional(category))
                }
            }
        }
    }

    /// Reads the events of the window from the store; on an error the last
    /// list stays.
    private func load() {
        let now = Int64(Date().timeIntervalSince1970)
        do {
            events = try store.events(nasId: nil, from: now - Int64(Self.windowDays) * 86_400, to: now)
            loadFailed = false
        } catch {
            loadFailed = true
        }
    }
}

/// One line of the chronicle.
private struct EventRowView: View {
    let row: EventRow

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: row.symbol)
                .foregroundStyle(row.level.color)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: row.title)
                Text(verbatim: row.detail)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Text(verbatim: row.time)
                .font(.footnote.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        // A finished problem steps back.
        .opacity(row.resolved ? 0.5 : 1)
        .accessibilityElement(children: .combine)
    }
}

// MARK: - Previews

/// Keychain stand-in without entries, so the previews never read real NAS.
private struct PreviewAccountStore: AccountStore {
    func loadAll() throws -> [StoredAccount] { [] }
    func save(_ stored: StoredAccount) throws {}
    func delete(id: UUID) throws {}
}

/// Store in demo mode on its own defaults suite, without queries.
@MainActor
private func previewStore() -> NasStore {
    let defaults = UserDefaults(suiteName: "nasstats-preview-events") ?? .standard
    defaults.set(true, forKey: NasStore.demoModeKey)
    return NasStore(
        accounts: NasAccounts(keychain: PreviewAccountStore(), core: NasCore()),
        defaults: defaults,
        demoSource: { demoSnapshots(now: $0, thresholds: $1) }
    )
}

#Preview("Ereignisse Demo hell") {
    EventsView()
        .environment(previewStore())
        .environment(\.locale, Locale(identifier: "de_DE"))
        .preferredColorScheme(.light)
}

#Preview("Ereignisse Demo dunkel") {
    EventsView()
        .environment(previewStore())
        .environment(\.locale, Locale(identifier: "de_DE"))
        .preferredColorScheme(.dark)
}
