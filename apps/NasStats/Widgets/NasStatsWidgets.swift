import SwiftUI
import WidgetKit

@main
struct NasStatsWidgets: WidgetBundle {
    var body: some Widget {
        StatusWidget()
    }
}

/// Placeholder widget until the app writes a snapshot to the App Group.
struct StatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "net.staude.nasstats.status", provider: StatusProvider()) { _ in
            StatusView()
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("NAS Stats")
        .description("Zustand Ihrer NAS.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

struct StatusEntry: TimelineEntry {
    let date: Date
}

struct StatusProvider: TimelineProvider {
    func placeholder(in context: Context) -> StatusEntry {
        StatusEntry(date: .now)
    }

    func getSnapshot(in context: Context, completion: @escaping (StatusEntry) -> Void) {
        completion(StatusEntry(date: .now))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<StatusEntry>) -> Void) {
        completion(Timeline(entries: [StatusEntry(date: .now)], policy: .never))
    }
}

struct StatusView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("NAS Stats")
                .font(.headline)
            Text("Noch keine Daten")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
