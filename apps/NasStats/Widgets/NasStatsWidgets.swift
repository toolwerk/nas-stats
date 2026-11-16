import AppIntents
import SwiftUI
import WidgetKit

@main
struct NasStatsWidgets: WidgetBundle {
    var body: some Widget {
        StatusWidget()
    }
}

/// Placeholder widget until #24: the NAS chosen in its settings (#26) or the
/// worst one, from the widget snapshot the app writes into the App Group (#23).
struct StatusWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "net.staude.nasstats.status", intent: SelectNasIntent.self, provider: StatusProvider()) { entry in
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
    /// The NAS the widget shows; `nil` without a snapshot or a NAS.
    let nas: WidgetNas?
}

struct StatusProvider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> StatusEntry {
        StatusEntry(date: .now, snapshot: nil, nas: nil)
    }

    func snapshot(for configuration: SelectNasIntent, in context: Context) async -> StatusEntry {
        let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL)
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

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            content
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private var content: some View {
        if let snapshot = entry.snapshot {
            if let nas = entry.nas {
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

/// The shown NAS: status dot, name, "DEMO", the error or the first warning,
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
