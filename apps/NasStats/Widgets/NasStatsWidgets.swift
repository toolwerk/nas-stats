import SwiftUI
import WidgetKit

@main
struct NasStatsWidgets: WidgetBundle {
    var body: some Widget {
        StatusWidget()
    }
}

/// Placeholder widget until #24: the worst NAS of the widget snapshot the
/// app writes into the App Group (#23).
struct StatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "net.staude.nasstats.status", provider: StatusProvider()) { entry in
            StatusView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("NAS Stats")
        .description("Zustand Ihrer NAS.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

struct StatusEntry: TimelineEntry {
    let date: Date
    /// `nil` before the app wrote a snapshot.
    let snapshot: WidgetSnapshot?
}

struct StatusProvider: TimelineProvider {
    func placeholder(in context: Context) -> StatusEntry {
        StatusEntry(date: .now, snapshot: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (StatusEntry) -> Void) {
        completion(StatusEntry(date: .now, snapshot: WidgetSnapshot.load(from: WidgetSnapshot.fileURL)))
    }

    /// Entries and reload after `WidgetSnapshot.timeline(now:)`; without a
    /// snapshot only an entry for now. The app reloads after every new state.
    func getTimeline(in context: Context, completion: @escaping (Timeline<StatusEntry>) -> Void) {
        let now = Date.now
        let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL)
        guard let snapshot else {
            completion(Timeline(entries: [StatusEntry(date: now, snapshot: nil)], policy: .never))
            return
        }
        let plan = snapshot.timeline(now: now)
        completion(Timeline(
            entries: plan.dates.map { StatusEntry(date: $0, snapshot: snapshot) },
            policy: plan.reloadAfter.map { .after($0) } ?? .never
        ))
    }
}

struct StatusView: View {
    let entry: StatusEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            content
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private var content: some View {
        if let snapshot = entry.snapshot {
            if let nas = snapshot.nas.first {
                NasLines(nas: nas, snapshot: snapshot, now: entry.date)
            } else {
                title
                note("Kein NAS eingerichtet")
            }
        } else {
            title
            note("Noch keine Daten")
        }
    }

    private var title: some View {
        Text("NAS Stats").font(.headline)
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.subheadline)
            .foregroundStyle(.secondary)
    }
}

/// The worst NAS: status dot, name, "DEMO", the error or the first warning,
/// and the time of the state once it is old.
private struct NasLines: View {
    let nas: WidgetNas
    let snapshot: WidgetSnapshot
    let now: Date

    var body: some View {
        HStack(spacing: 6) {
            StatusDot(level: nas.status)
            Text(nas.name)
                .font(.headline)
                .lineLimit(1)
            Spacer(minLength: 0)
            if snapshot.demo {
                Text("DEMO")
                    .font(.caption2.bold())
                    .foregroundStyle(.secondary)
            }
        }
        // The error first: it explains a red or dashed dot, while the
        // findings may be those of an old state.
        if let problem = nas.problemLine {
            Text(problem)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(3)
        } else if let finding = nas.firstFinding {
            Text(finding)
                .font(.subheadline)
                .foregroundStyle(nas.firstFindingLevel?.color ?? .primary)
                .lineLimit(3)
        }
        Spacer(minLength: 0)
        if let observedAt = nas.observedAt, snapshot.isStale(nas, now: now) {
            Text(StatusFormat.observed(observedAt, now: now))
                .font(.caption)
                .foregroundStyle(StatusLevel.offline.color)
        }
    }
}
