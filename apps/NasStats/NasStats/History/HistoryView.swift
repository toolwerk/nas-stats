import Charts
import SwiftUI
#if os(macOS)
import AppKit
#endif

/// History screen (#19): fill level of every volume with the forecast,
/// disk temperatures, CPU and RAM and the network traffic of one NAS over a
/// range. Hourly and daily values show as a band from min to max with the
/// mean as a line.
struct HistoryView: View {
    @Environment(NasStore.self) private var store
    @SceneStorage("historyNasId") private var nasId = ""
    @SceneStorage("historyRangeDays") private var rangeDays = HistoryRange.month.days

    /// What the last load produced.
    private enum Loaded {
        case content(HistoryContent)
        case failed(String)
    }

    @State private var loaded: Loaded?
    /// Error of a CSV export (#45), shown as an alert.
    @State private var csvError: String?

    private var entry: NasEntry? {
        store.entries.first { $0.id == nasId } ?? store.entries.first
    }

    private var range: HistoryRange { HistoryRange(days: rangeDays) ?? .month }

    /// Everything a load depends on.
    private struct LoadKey: Hashable {
        let nasId: String?
        let days: Int
        let observedAt: Int64?
        let demo: Bool
        /// History taken over from the collector (#124).
        let revision: Int
    }

    private var loadKey: LoadKey {
        LoadKey(
            nasId: entry?.id, days: range.days, observedAt: entry?.snapshot?.observedAt,
            demo: store.demoMode, revision: store.historyRevision)
    }

    var body: some View {
        NavigationStack {
            Group {
                if let entry {
                    screen(entry)
                } else {
                    ContentUnavailableView("Kein NAS", systemImage: "externaldrive.badge.questionmark")
                }
            }
            .navigationTitle("Verlauf")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if let entry, entry.snapshot != nil {
                    ToolbarItem(placement: .primaryAction) { exportMenu(entry) }
                }
                ToolbarItem(placement: .primaryAction) { nasMenu }
            }
            .alert(
                "CSV",
                isPresented: Binding(get: { csvError != nil }, set: { if !$0 { csvError = nil } })
            ) {
                Button("OK") {}
            } message: {
                Text(verbatim: csvError ?? "")
            }
        }
    }

    // MARK: - CSV export (#45)

    private func export(_ kind: CSVExport.Kind, of entry: NasEntry) -> CSVExport {
        let id = entry.id
        let range = range
        let store = store
        let error = $csvError
        return CSVExport(
            fileName: CSVExport.fileName(
                kind: kind, nasName: entry.displayName, range: range, demo: store.demoMode),
            make: {
                do {
                    switch kind {
                    case .history: return try store.historyCSV(nasId: id, range: range)
                    case .events: return try store.eventsCSV(nasId: id, range: range)
                    }
                } catch let failure {
                    error.wrappedValue = failure.localizedDescription
                    throw failure
                }
            })
    }

    private func exportMenu(_ entry: NasEntry) -> some View {
        let history = export(.history, of: entry)
        let events = export(.events, of: entry)
        return Menu("Export", systemImage: "square.and.arrow.up") {
            ShareLink(
                "Verlauf als CSV teilen…", item: history, preview: SharePreview(history.fileName))
            ShareLink(
                "Ereignisse als CSV teilen…", item: events, preview: SharePreview(events.fileName))
            #if os(macOS)
            Divider()
            Button("Verlauf sichern…") { save(history) }
            Button("Ereignisse sichern…") { save(events) }
            #endif
        }
    }

    #if os(macOS)
    /// The Mac share menu offers no folder, so the file goes through a save
    /// panel.
    private func save(_ export: CSVExport) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.commaSeparatedText]
        panel.nameFieldStringValue = export.fileName
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            let text = try export.make()
            try Data(text.utf8).write(to: url, options: .atomic)
        } catch {
            csvError = error.localizedDescription
        }
    }
    #endif

    private var nasMenu: some View {
        Menu {
            Picker("NAS", selection: Binding(get: { entry?.id ?? "" }, set: { nasId = $0 })) {
                ForEach(store.entries) { entry in
                    Label {
                        Text(verbatim: entry.displayName)
                    } icon: {
                        StatusDot(level: entry.statusLevel)
                    }
                    .tag(entry.id)
                }
            }
        } label: {
            Text(verbatim: entry?.displayName ?? "NAS")
        }
    }

    @ViewBuilder
    private func screen(_ entry: NasEntry) -> some View {
        if entry.snapshot == nil {
            ContentUnavailableView("Noch kein Stand", systemImage: "hourglass")
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Picker("Zeitraum", selection: Binding(get: { range }, set: { rangeDays = $0.days })) {
                        ForEach(HistoryRange.options, id: \.days) { range in
                            Text(range.label).tag(range)
                        }
                    }
                    .pickerStyle(.segmented)
                    gapsHint
                    loadedBody(loaded)
                    if store.demoMode {
                        Text("Demo-Werte")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding()
            }
            .task(id: loadKey) { load(entry) }
        }
    }

    /// The iPhone cannot tell whether a Mac collects (#37), so the hint
    /// stands there whenever the values are real.
    @ViewBuilder
    private var gapsHint: some View {
        #if os(iOS)
        if !store.demoMode {
            Label(
                "Verlauf mit Lücken, für lückenlose Aufzeichnung NAS Stats auf einem Mac laufen lassen",
                systemImage: "info.circle"
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
        #endif
    }

    @ViewBuilder
    private func loadedBody(_ loaded: Loaded?) -> some View {
        switch loaded {
        case nil:
            ProgressView().frame(maxWidth: .infinity)
        case let .failed(message)?:
            Text(verbatim: "Verlauf nicht lesbar: \(message)")
                .font(.footnote)
                .foregroundStyle(.red)
        case let .content(content)?:
            if content.isEmpty {
                ContentUnavailableView("Noch kein Verlauf", systemImage: "chart.xyaxis.line")
            } else {
                charts(content)
            }
        }
    }

    private func load(_ entry: NasEntry) {
        guard let snapshot = entry.snapshot else { return }
        let id = entry.id
        do {
            loaded = .content(
                try HistoryContent(
                    snapshot: snapshot, range: range, now: Date(),
                    load: { try store.history(nasId: id, metric: $0, range: $1) },
                    forecast: { try store.forecast(nasId: id, volumeId: $0) }))
        } catch {
            loaded = .failed(error.localizedDescription)
        }
    }

    // MARK: - Charts

    @ViewBuilder
    private func charts(_ content: HistoryContent) -> some View {
        ForEach(content.volumes) { volumeCard($0) }
        if !content.temperatures.isEmpty {
            card("Plattentemperaturen") {
                LineChartView(lines: content.temperatures, unit: "°C")
            }
        }
        if !content.load.isEmpty {
            card("CPU und RAM") {
                LineChartView(lines: content.load, unit: "%")
            }
        }
        ForEach(content.networks) { network in
            card("Netz \(network.id)") {
                LineChartView(lines: network.lines, unit: "MB/s")
            }
        }
    }

    private func card<Content: View>(
        _ title: String, trailing: String? = nil, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(verbatim: title).font(.headline)
                Spacer()
                if let trailing {
                    Text(verbatim: trailing).font(.footnote).foregroundStyle(.secondary)
                }
            }
            content()
        }
        .padding()
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 12))
    }

    private func volumeCard(_ chart: HistoryContent.VolumeChart) -> some View {
        let growth = chart.growthPercentPerDay.map {
            "+\($0.formatted(.number.precision(.fractionLength(2)))) % pro Tag"
        }
        return card("Belegung \(chart.name)", trailing: growth) {
            VolumeChartView(chart: chart)
            if let text = chart.forecastText {
                Text(verbatim: [text.main, text.date].compactMap { $0 }.joined(separator: " · "))
                    .font(.footnote)
            } else if let days = chart.historyDays {
                Text(verbatim: "Prognose ab \(HistoryContent.minimumForecastDays) Tagen Verlauf, bisher \(days)")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// Band from min to max and line of the mean, one colour per line.
private struct LineChartView: View {
    let lines: [HistoryContent.Line]
    let unit: String

    var body: some View {
        Chart {
            ForEach(lines) { line in
                marks(of: line)
            }
        }
        .chartYAxisLabel(unit)
        .frame(height: 160)
    }

    @ChartContentBuilder
    private func marks(of line: HistoryContent.Line) -> some ChartContent {
        ForEach(line.points, id: \.at) { point in
            AreaMark(
                x: .value("Zeit", point.at),
                yStart: .value("Min", point.min),
                yEnd: .value("Max", point.max),
                series: .value("Reihe", line.id)
            )
            .foregroundStyle(by: .value("Name", line.name))
            .opacity(0.2)
            LineMark(
                x: .value("Zeit", point.at),
                y: .value("Mittel", point.mean),
                series: .value("Reihe", line.id)
            )
            .foregroundStyle(by: .value("Name", line.name))
            if line.points.count == 1 {
                PointMark(x: .value("Zeit", point.at), y: .value("Mittel", point.mean))
                    .foregroundStyle(by: .value("Name", line.name))
            }
        }
    }
}

/// Fill level in percent: band, mean, red line at 100 % and the dashed
/// forecast.
private struct VolumeChartView: View {
    let chart: HistoryContent.VolumeChart

    /// Lower edge of the axis: the lowest value less some room, so the
    /// band does not hug the bottom.
    private var lowerBound: Double {
        let lowest = (chart.line.points.map(\.min) + chart.forecast.map(\.percent)).min() ?? 0
        return Swift.max(0, ((lowest - 5) / 10).rounded(.down) * 10)
    }

    /// 100 %, or more where a volume reports more used than its size.
    private var upperBound: Double {
        Swift.max(100, chart.line.points.map(\.max).max() ?? 100)
    }

    var body: some View {
        Chart {
            ForEach(chart.line.points, id: \.at) { point in
                AreaMark(
                    x: .value("Zeit", point.at),
                    yStart: .value("Min", point.min),
                    yEnd: .value("Max", point.max),
                    series: .value("Reihe", "Belegung")
                )
                .foregroundStyle(.tint)
                .opacity(0.2)
                LineMark(
                    x: .value("Zeit", point.at),
                    y: .value("Mittel", point.mean),
                    series: .value("Reihe", "Belegung")
                )
                .foregroundStyle(.tint)
                if chart.line.points.count == 1 {
                    PointMark(x: .value("Zeit", point.at), y: .value("Mittel", point.mean))
                        .foregroundStyle(.tint)
                }
            }
            ForEach(Array(chart.forecast.enumerated()), id: \.offset) { _, point in
                LineMark(
                    x: .value("Zeit", point.at),
                    y: .value("Prognose", point.percent),
                    series: .value("Reihe", "Prognose")
                )
                .foregroundStyle(.secondary)
                .lineStyle(StrokeStyle(lineWidth: 2, dash: [4, 4]))
            }
            RuleMark(y: .value("Voll", 100))
                .foregroundStyle(.red)
                .lineStyle(StrokeStyle(lineWidth: 1, dash: [3, 3]))
        }
        .chartYScale(domain: lowerBound...upperBound)
        .chartYAxisLabel("%")
        .frame(height: 180)
    }
}

// MARK: - Previews

/// Keychain stand-in without entries, so the previews never read real NAS.
private struct PreviewAccountStore: AccountStore {
    func loadAll() throws -> [StoredAccount] { [] }
    func save(_ stored: StoredAccount) throws {}
    func delete(id: UUID) throws {}
}

/// Store with the demo data set, on its own defaults suite, without queries.
@MainActor
private func previewStore() -> NasStore {
    let defaults = UserDefaults(suiteName: "nasstats-preview-history") ?? .standard
    defaults.set(true, forKey: NasStore.demoModeKey)
    return NasStore(
        accounts: NasAccounts(keychain: PreviewAccountStore()),
        defaults: defaults,
        demoSource: { demoSnapshots(now: $0, thresholds: $1) }
    )
}

#Preview("Verlauf Demo hell") {
    HistoryView()
        .environment(previewStore())
        .environment(\.locale, Locale(identifier: "de_DE"))
        .preferredColorScheme(.light)
}

#Preview("Verlauf Demo dunkel") {
    HistoryView()
        .environment(previewStore())
        .environment(\.locale, Locale(identifier: "de_DE"))
        .preferredColorScheme(.dark)
}
