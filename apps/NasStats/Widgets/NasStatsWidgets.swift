import AppIntents
import SwiftUI
import WidgetKit

@main
struct NasStatsWidgets: WidgetBundle {
    var body: some Widget {
        StatusWidget()
    }
}

/// The widget „NAS“ (#24): small with the ring of the fullest volume, medium
/// with bars per volume and key figures. It shows the NAS chosen in its
/// settings (#26) or the worst one, from the widget snapshot the app writes
/// into the App Group (#23). The kind stays, so placed widgets keep their
/// choice.
struct StatusWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "net.staude.nasstats.status", intent: SelectNasIntent.self, provider: StatusProvider()) { entry in
            StatusView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("NAS")
        .description("Zustand, Füllstand und Kennzahlen eines NAS.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

struct StatusEntry: TimelineEntry {
    let date: Date
    /// `nil` before the app wrote a snapshot.
    let snapshot: WidgetSnapshot?
    /// The NAS the widget shows; `nil` without a snapshot or a NAS.
    let nas: WidgetNas?
}

extension StatusEntry {
    /// Demo values for the gallery and the placeholder.
    static var sample: StatusEntry { .init(entry: WidgetSamples.demo, now: WidgetSamples.now) }

    init(entry snapshot: WidgetSnapshot, now: Date) {
        self.init(date: now, snapshot: snapshot, nas: snapshot.nas.first)
    }
}

struct StatusProvider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> StatusEntry {
        .sample
    }

    func snapshot(for configuration: SelectNasIntent, in context: Context) async -> StatusEntry {
        let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL)
        // The gallery shows demo values until the app wrote a state.
        if snapshot == nil, context.isPreview { return .sample }
        return StatusEntry(
            date: .now, snapshot: snapshot, nas: snapshot?.selected(configuration.nas?.id)
        )
    }

    /// Entries and reload after `WidgetSnapshot.timeline(for:now:)` for the
    /// shown NAS; without a snapshot only an entry for now. The app reloads
    /// after every new state.
    func timeline(for configuration: SelectNasIntent, in context: Context) async -> Timeline<StatusEntry> {
        let now = Date.now
        guard let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL) else {
            return Timeline(entries: [StatusEntry(date: now, snapshot: nil, nas: nil)], policy: .never)
        }
        let shown = snapshot.selected(configuration.nas?.id)
        let plan = snapshot.timeline(for: shown, now: now)
        return Timeline(
            entries: plan.dates.map { StatusEntry(date: $0, snapshot: snapshot, nas: shown) },
            policy: plan.reloadAfter.map { .after($0) } ?? .never
        )
    }
}

struct StatusView: View {
    let entry: StatusEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private var content: some View {
        if let snapshot = entry.snapshot {
            if let nas = entry.nas {
                if family == .systemMedium {
                    NasMediumView(nas: nas, snapshot: snapshot, now: entry.date)
                } else {
                    NasSmallView(nas: nas, snapshot: snapshot, now: entry.date)
                }
            } else {
                empty("Kein NAS eingerichtet")
            }
        } else {
            empty("Noch keine Daten")
        }
    }

    private func empty(_ text: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("NAS Stats").font(.headline)
            Text(text)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

private let previews: [(String, WidgetSnapshot)] = [
    ("Demo", WidgetSamples.demo), ("Gesund", WidgetSamples.healthy),
    ("Nicht erreichbar", WidgetSamples.offline), ("Alter Stand", WidgetSamples.old),
    ("Ohne Volumes", WidgetSamples.noVolumes),
]

#Preview("Klein", as: .systemSmall) {
    StatusWidget()
} timeline: {
    for (_, snapshot) in previews { StatusEntry(entry: snapshot, now: WidgetSamples.now) }
}

#Preview("Mittel", as: .systemMedium) {
    StatusWidget()
} timeline: {
    for (_, snapshot) in previews { StatusEntry(entry: snapshot, now: WidgetSamples.now) }
}
