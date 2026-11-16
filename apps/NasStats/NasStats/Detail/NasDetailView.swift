import SwiftUI

/// Detail screen of one NAS (#18): head, storage, disks, backups, system,
/// network and containers of the last snapshot. Follows the store, so every
/// new query shows at once. Values the vendor does not report are "n/a",
/// empty lists and a missing UPS leave out their section.
struct NasDetailView: View {
    let id: String

    @Environment(NasStore.self) private var store
    @Environment(\.locale) private var locale
    @Environment(\.timeZone) private var timeZone

    private var entry: NasEntry? { store.entries.first { $0.id == id } }

    var body: some View {
        Group {
            if let entry {
                content(entry)
            } else {
                // Removed meanwhile, or the demo mode was switched.
                ContentUnavailableView("NAS nicht mehr vorhanden", systemImage: "externaldrive.badge.xmark")
            }
        }
        .navigationTitle(Text(verbatim: entry?.displayName ?? ""))
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar {
            if let entry, let url = URL(string: entry.baseUrl) {
                ToolbarItem(placement: .primaryAction) {
                    Link("Web-UI", destination: url)
                }
            }
        }
    }

    private func content(_ entry: NasEntry) -> some View {
        // Ages and days ("vor 9 h", "morgen", the date of an old snapshot)
        // follow the clock even while no new query arrives.
        TimelineView(.everyMinute) { context in
            List {
                Section {
                    header(entry, now: context.date)
                }
                if let snapshot = entry.snapshot {
                    sections(
                        snapshot, content: NasDetailContent(snapshot: snapshot),
                        thresholds: entry.thresholds, now: context.date)
                }
            }
        }
    }

    // MARK: - Head

    private func header(_ entry: NasEntry, now: Date) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                StatusDot(level: entry.statusLevel, size: 10)
                Text(verbatim: entry.displayName)
                    .font(.title3.weight(.semibold))
                Spacer(minLength: 8)
                if entry.snapshot?.demo == true {
                    chip("DEMO", color: .secondary)
                }
                chip(entry.statusLevel.label, color: entry.statusLevel.color)
            }
            Text(verbatim: "Modell \(entry.displayModel ?? StatusFormat.notAvailable) · Firmware \(entry.snapshot?.nas.firmware ?? StatusFormat.notAvailable)")
                .font(.footnote)
                .foregroundStyle(.secondary)
            if let snapshot = entry.snapshot {
                Text(verbatim: "Laufzeit \(StatusFormat.uptime(snapshot.system.uptimeSeconds))")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                if snapshot.system.updateAvailable == true {
                    let version = snapshot.system.updateVersion.map { ": \($0)" } ?? ""
                    Text(verbatim: "Update verfügbar\(version)")
                        .font(.footnote)
                        .foregroundStyle(.tint)
                }
                Text(verbatim: StatusFormat.observed(
                    Date(timeIntervalSince1970: TimeInterval(snapshot.observedAt)),
                    now: now, locale: locale, timeZone: timeZone))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else {
                Text("Noch kein Stand")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            if let error = entry.localError ?? entry.problem?.message ?? entry.queryError {
                Text(verbatim: error)
                    .font(.footnote)
                    .foregroundStyle(.red)
            }
        }
        .padding(.vertical, 4)
    }

    private func chip(_ text: String, color: some ShapeStyle) -> some View {
        Text(verbatim: text)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .foregroundStyle(color)
            .overlay(Capsule().strokeBorder(color, lineWidth: 1))
    }

    // MARK: - Sections

    @ViewBuilder
    private func sections(
        _ snapshot: Snapshot, content: NasDetailContent, thresholds: AlertThresholds, now: Date
    ) -> some View {
        if !content.volumes.isEmpty || !content.pools.isEmpty {
            Section("Speicher") {
                ForEach(content.volumes, id: \.id) { volume in
                    volumeRow(volume, thresholds: thresholds.fill)
                }
                ForEach(content.pools, id: \.id) { pool in
                    poolRow(pool)
                }
            }
        }
        if !snapshot.disks.isEmpty {
            Section("Platten") {
                ForEach(snapshot.disks, id: \.id) { disk in
                    diskRow(disk, warningC: thresholds.diskTempWarningC)
                }
            }
        }
        switch content.backups {
        case let .tasks(tasks):
            Section("Backups") {
                ForEach(tasks, id: \.id) { task in
                    backupRow(task, now: now)
                }
            }
        case .interfaceOpen:
            Section("Backups") {
                Text("Hybrid Backup Sync: Schnittstelle offen")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        case .hidden:
            EmptyView()
        }
        Section("System") {
            row(
                "CPU · RAM",
                detail: StatusFormat.bytes(snapshot.system.memoryTotalBytes, locale: locale),
                value: Text(verbatim: "\(percent(snapshot.system.cpuPercent)) · \(percent(snapshot.system.memoryPercent))"))
            if let ups = snapshot.ups {
                upsRow(ups)
            }
        }
        if !snapshot.interfaces.isEmpty {
            Section("Netz") {
                ForEach(snapshot.interfaces, id: \.name) { interface in
                    row(
                        interface.name,
                        value: Text(verbatim: StatusFormat.throughput(
                            rx: interface.rxBytesPerSec, tx: interface.txBytesPerSec, locale: locale)))
                }
            }
        }
        if !content.containers.isEmpty {
            Section {
                ForEach(content.containers, id: \.name) { container in
                    row(
                        container.name,
                        value: Text(container.running ? "läuft" : "gestoppt")
                            .foregroundStyle(container.running ? Color.secondary : Color.primary))
                }
            } header: {
                HStack {
                    Text("Container")
                    Spacer()
                    if let summary = content.containerSummary {
                        Text(verbatim: summary)
                    }
                }
            }
        }
    }

    private func volumeRow(_ volume: Volume, thresholds: FillThresholds) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            // The forecast follows with #60 and #67.
            FillBar(
                name: volume.name, percent: volume.usedPercent,
                usedBytes: volume.usedBytes, totalBytes: volume.totalBytes,
                forecast: nil, thresholds: thresholds)
            if volume.health == .warning || volume.health == .critical {
                Text(verbatim: volume.statusText ?? partLabel(volume.health))
                    .font(.caption)
                    .foregroundStyle(partLevel(volume.health).color)
            }
        }
        .padding(.vertical, 2)
    }

    private func poolRow(_ pool: Volume) -> some View {
        let subtitle = [pool.raid, pool.statusText].compactMap { $0 }.joined(separator: " · ")
        return row(
            pool.name,
            detail: subtitle.isEmpty ? nil : subtitle,
            value: Text(verbatim: partLabel(pool.health))
                .foregroundStyle(partLevel(pool.health).color))
    }

    private func diskRow(_ disk: Disk, warningC: Double) -> some View {
        var detail = "\(disk.model ?? StatusFormat.notAvailable) · \(StatusFormat.hours(disk.powerOnHours, locale: locale))"
        if disk.spinning == false {
            detail += " · Ruhezustand"
        }
        let hot = (disk.temperatureC ?? 0) >= warningC
        let temperature = Text(verbatim: StatusFormat.temperature(disk.temperatureC, locale: locale))
            .foregroundStyle(hot ? StatusLevel.warning.color : Color.primary)
        let smart = Text(verbatim: disk.smartText ?? partLabel(disk.health))
            .foregroundStyle(partLevel(disk.health).color)
        return row(disk.name, detail: detail, value: Text("\(temperature)  \(smart)"))
    }

    private func backupRow(_ task: BackupTask, now: Date) -> some View {
        let next = StatusFormat.nextRun(
            task.nextRun.map { Date(timeIntervalSince1970: TimeInterval($0)) },
            now: now, locale: locale, timeZone: timeZone)
        let value: Text = switch task.lastResult {
        case .ok, nil:
            Text(verbatim: StatusFormat.backupAge(task.lastRun.map { Date(timeIntervalSince1970: TimeInterval($0)) }, now: now))
        case .error:
            Text("Fehler").foregroundStyle(StatusLevel.critical.color)
        case .running:
            Text("läuft")
        case .never:
            Text("nie")
        }
        return row(task.name, detail: "\(backupKindLabel(task.kind)) · nächster Lauf \(next)", value: value)
    }

    private func upsRow(_ ups: Ups) -> some View {
        let state: Text = switch ups.state {
        case .online: Text("Netzbetrieb")
        case .onBattery: Text("Batteriebetrieb").foregroundStyle(StatusLevel.critical.color)
        case nil: Text(verbatim: StatusFormat.notAvailable)
        }
        let value = "\(percent(ups.chargePercent)) · \(StatusFormat.runtime(ups.runtimeSeconds))"
        return VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline) {
                Text("USV")
                Spacer(minLength: 8)
                Text(verbatim: value).monospacedDigit()
            }
            state
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    /// Name with an optional grey line below and the value on the right.
    private func row(_ title: String, detail: String? = nil, value: Text) -> some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: title)
                if let detail {
                    Text(verbatim: detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 8)
            value
                .monospacedDigit()
                .multilineTextAlignment(.trailing)
        }
    }

    private func percent(_ value: Double?) -> String {
        StatusFormat.percent(value, fractionDigits: 0, locale: locale)
    }
}

#Preview("NAS-Detail") {
    NasDetailPreview()
        .preferredColorScheme(.light)
}

#Preview("NAS-Detail dunkel") {
    NasDetailPreview()
        .preferredColorScheme(.dark)
}

/// The demo NAS Keller; demo values only, never real NAS data.
private struct NasDetailPreview: View {
    @State private var store: NasStore = {
        let defaults = UserDefaults(suiteName: "preview-detail") ?? .standard
        defaults.set(true, forKey: NasStore.demoModeKey)
        return NasStore(
            accounts: NasAccounts(),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: nil)
    }()

    var body: some View {
        NavigationStack {
            NasDetailView(id: "demo-keller")
        }
        .environment(store)
        .environment(\.locale, Locale(identifier: "de_DE"))
    }
}
