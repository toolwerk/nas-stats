#if os(macOS)
import SwiftUI

/// Popover of the menu bar symbol (#28): a segment per NAS with its status
/// dot, the card of the chosen NAS as on the iPhone, its latest open event
/// and a footer with the age of the state, "Fenster öffnen" and
/// "Aktualisieren". Starts at the worst NAS on every opening, since the
/// status item builds a new view each time.
struct MenuBarPopover: View {
    /// How far back the event line looks, like the events tab.
    private static let eventWindowDays = 30

    var onOpenWindow: () -> Void = {}
    var onAddNas: () -> Void = {}

    @Environment(NasStore.self) private var store
    @State private var chosen: String?
    @State private var event: NasEvent?

    private var shownId: String? {
        PopoverContent.shownId(entries: store.entries, chosen: chosen)
    }

    private var shown: NasEntry? {
        store.entries.first { $0.id == shownId }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if store.entries.isEmpty {
                empty
            } else {
                head
                TimelineView(.everyMinute) { context in
                    VStack(alignment: .leading, spacing: 12) {
                        if let shown {
                            NasCard(entry: shown, now: context.date, intervalMinutes: store.queryIntervalMinutes)
                        }
                        if let event {
                            eventLine(event, now: context.date)
                        }
                        footer(now: context.date)
                    }
                }
            }
        }
        .padding(12)
        .frame(width: 360, alignment: .leading)
        // Loads again for another NAS, after every new state and when the
        // demo mode is switched.
        .task(id: LoadKey(nasId: shownId, observedAt: shown?.observedAt, demo: store.demoMode)) {
            loadEvent()
        }
    }

    private struct LoadKey: Hashable {
        let nasId: String?
        let observedAt: Int64?
        let demo: Bool
    }

    private func loadEvent() {
        guard let shownId else {
            event = nil
            return
        }
        let now = Int64(Date().timeIntervalSince1970)
        // No upper bound: `startedAt` is the clock of the NAS, which may run ahead.
        let events = try? store.events(
            nasId: shownId, from: now - Int64(Self.eventWindowDays) * 86_400, to: .max
        )
        event = events.flatMap { PopoverContent.latestOpenEvent($0, nasId: shownId) }
    }

    // MARK: - Head

    @ViewBuilder
    private var head: some View {
        let entries = store.entries
        if entries.count > PopoverContent.maxSegments {
            Picker("NAS", selection: Binding(
                get: { shownId ?? "" },
                set: { chosen = $0 }
            )) {
                ForEach(entries) { entry in
                    Label {
                        Text(verbatim: entry.displayName)
                    } icon: {
                        Image(systemName: "circle.fill")
                            .foregroundStyle(entry.statusLevel.color)
                    }
                    .tag(entry.id)
                }
            }
            .pickerStyle(.menu)
            .labelsHidden()
        } else if entries.count > 1 {
            segments(entries)
        }
    }

    /// Equally wide buttons, the shown NAS highlighted.
    private func segments(_ entries: [NasEntry]) -> some View {
        HStack(spacing: 2) {
            ForEach(entries) { entry in
                let selected = entry.id == shownId
                Button {
                    chosen = entry.id
                } label: {
                    HStack(spacing: 6) {
                        StatusDot(level: entry.statusLevel, size: 6)
                        Text(verbatim: entry.displayName)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                    .font(.callout.weight(selected ? .semibold : .regular))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 4)
                    .padding(.horizontal, 6)
                    .background(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(selected ? Color.cardBackground : Color.clear)
                            .shadow(color: .black.opacity(selected ? 0.15 : 0), radius: 1, y: 0.5)
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? .isSelected : [])
            }
        }
        .padding(2)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.secondary.opacity(0.15))
        )
    }

    // MARK: - Event and footer

    private func eventLine(_ event: NasEvent, now: Date) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: event.text)
                .font(.callout.weight(.semibold))
                .foregroundStyle(EventsContent.level(of: event).color)
            Text(verbatim: PopoverContent.openSince(event, now: now))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    private func footer(now: Date) -> some View {
        HStack(spacing: 8) {
            Text(verbatim: PopoverContent.footer(entry: shown, demo: store.demoMode, now: now))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 4)
            Button("Fenster öffnen", action: onOpenWindow)
            Button("Aktualisieren") {
                Task { await store.refreshAll() }
            }
        }
        .controlSize(.small)
    }

    // MARK: - Without NAS

    private var empty: some View {
        VStack(spacing: 10) {
            Image(systemName: "externaldrive")
                .font(.system(size: 32))
                .foregroundStyle(.tint)
            if let loadError = store.loadError {
                Text(verbatim: loadError)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
            } else {
                Text("Noch kein NAS verbunden")
                    .foregroundStyle(.secondary)
            }
            Button("NAS hinzufügen …", action: onAddNas)
                .buttonStyle(.borderedProminent)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
    }
}
#endif
