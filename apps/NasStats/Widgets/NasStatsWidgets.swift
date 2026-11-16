import AppIntents
import SwiftUI
import WidgetKit

@main
struct NasStatsWidgets: WidgetBundle {
    var body: some Widget {
        StatusWidget()
        FillWidget()
        AllNasWidget()
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
        .supportedFamilies(Self.families)
    }
}

extension StatusWidget {
    /// The lock screen shapes (#25) exist on iPhone only.
    static var families: [WidgetFamily] {
        #if os(iOS)
        [.systemSmall, .systemMedium, .accessoryCircular, .accessoryRectangular, .accessoryInline]
        #else
        [.systemSmall, .systemMedium]
        #endif
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
                switch family {
                case .systemMedium:
                    NasMediumView(nas: nas, snapshot: snapshot, now: entry.date)
                #if os(iOS)
                case .accessoryCircular:
                    NasCircularView(nas: nas, demo: snapshot.demo)
                case .accessoryRectangular:
                    NasRectangularView(nas: nas, snapshot: snapshot, now: entry.date)
                case .accessoryInline:
                    NasInlineView(nas: nas, demo: snapshot.demo)
                #endif
                default:
                    NasSmallView(nas: nas, snapshot: snapshot, now: entry.date)
                }
            } else {
                WidgetEmptyView(text: "Kein NAS eingerichtet")
            }
        } else {
            WidgetEmptyView(text: "Noch keine Daten")
        }
    }
}

/// Text of a widget without a NAS or without a snapshot.
struct WidgetEmptyView: View {
    let text: String
    @Environment(\.widgetFamily) private var family

    var body: some View {
        switch family {
        #if os(iOS)
        case .accessoryCircular:
            NasCircularView(nas: nil, demo: false)
        case .accessoryInline:
            Text("NAS Stats · \(text)")
        #endif
        default:
            VStack(alignment: .leading, spacing: 4) {
                Text("NAS Stats").font(.headline)
                Text(text)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
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

/// The widget „Füllstand“ (#38): small, a bar per volume of the NAS chosen
/// in its settings (#26) with the sum of the sizes below.
struct FillWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "net.staude.nasstats.fill", intent: SelectNasIntent.self, provider: StatusProvider()) { entry in
            FillView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Füllstand")
        .description("Füllstand der Volumes eines NAS.")
        .supportedFamilies([.systemSmall])
    }
}

struct FillView: View {
    let entry: StatusEntry

    var body: some View {
        if let snapshot = entry.snapshot {
            if let nas = entry.nas {
                FillSmallView(nas: nas, snapshot: snapshot, now: entry.date)
            } else {
                WidgetEmptyView(text: "Kein NAS eingerichtet")
            }
        } else {
            WidgetEmptyView(text: "Noch keine Daten")
        }
    }
}

/// The widget „Alle NAS“ (#38): large, a row per NAS and the history of the
/// fullest volume. It has no setting, it shows every NAS of the snapshot.
struct AllNasWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "net.staude.nasstats.all", provider: AllNasProvider()) { entry in
            AllNasView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Alle NAS")
        .description("Zustand und Füllstand aller NAS.")
        .supportedFamilies([.systemLarge])
    }
}

struct AllNasEntry: TimelineEntry {
    let date: Date
    /// `nil` before the app wrote a snapshot.
    let snapshot: WidgetSnapshot?
}

struct AllNasProvider: TimelineProvider {
    func placeholder(in context: Context) -> AllNasEntry {
        AllNasEntry(date: WidgetSamples.now, snapshot: WidgetSamples.all)
    }

    func getSnapshot(in context: Context, completion: @escaping (AllNasEntry) -> Void) {
        let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL)
        // The gallery shows demo values until the app wrote a state.
        if snapshot == nil, context.isPreview {
            completion(placeholder(in: context))
        } else {
            completion(AllNasEntry(date: .now, snapshot: snapshot))
        }
    }

    /// Entries and reload after `WidgetSnapshot.timeline(for:now:)` for the
    /// NAS in the rows; without a snapshot only an entry for now.
    func getTimeline(in context: Context, completion: @escaping (Timeline<AllNasEntry>) -> Void) {
        let now = Date.now
        guard let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL) else {
            completion(Timeline(entries: [AllNasEntry(date: now, snapshot: nil)], policy: .never))
            return
        }
        let plan = snapshot.timeline(for: snapshot.largeRows, now: now)
        completion(Timeline(
            entries: plan.dates.map { AllNasEntry(date: $0, snapshot: snapshot) },
            policy: plan.reloadAfter.map { .after($0) } ?? .never))
    }
}

struct AllNasView: View {
    let entry: AllNasEntry

    var body: some View {
        if let snapshot = entry.snapshot {
            if snapshot.nas.isEmpty {
                WidgetEmptyView(text: "Kein NAS eingerichtet")
            } else {
                AllNasLargeView(snapshot: snapshot, now: entry.date)
            }
        } else {
            WidgetEmptyView(text: "Noch keine Daten")
        }
    }
}

#Preview("Füllstand", as: .systemSmall) {
    FillWidget()
} timeline: {
    for (_, snapshot) in previews { StatusEntry(entry: snapshot, now: WidgetSamples.now) }
}

#Preview("Alle NAS", as: .systemLarge) {
    AllNasWidget()
} timeline: {
    AllNasEntry(date: WidgetSamples.now, snapshot: WidgetSamples.all)
    AllNasEntry(date: WidgetSamples.now, snapshot: WidgetSamples.allWithoutHistory)
}

#if os(iOS)
#Preview("Rund", as: .accessoryCircular) {
    StatusWidget()
} timeline: {
    for (_, snapshot) in previews { StatusEntry(entry: snapshot, now: WidgetSamples.now) }
}

#Preview("Rechteckig", as: .accessoryRectangular) {
    StatusWidget()
} timeline: {
    for (_, snapshot) in previews { StatusEntry(entry: snapshot, now: WidgetSamples.now) }
}

#Preview("Zeile", as: .accessoryInline) {
    StatusWidget()
} timeline: {
    for (_, snapshot) in previews { StatusEntry(entry: snapshot, now: WidgetSamples.now) }
}
#endif
