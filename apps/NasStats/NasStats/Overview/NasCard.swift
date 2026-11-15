import SwiftUI

/// Card of one NAS in the overview (#17): status dot, name, model and
/// vendor, the findings as lines in their colour, a fill bar per volume and
/// the key figures. Without a snapshot only the head and the error line.
struct NasCard: View {
    let entry: NasEntry
    var now: Date = .now

    /// Problem line and findings, in this order.
    private var lines: [(text: String, level: StatusLevel)] {
        var lines: [(String, StatusLevel)] = []
        if let problem = entry.problemLine {
            lines.append((problem, entry.statusLevel == .offline ? .offline : entry.statusLevel))
        }
        for finding in entry.snapshot?.findings ?? [] {
            lines.append((finding.text, finding.level == .critical ? .critical : .warning))
        }
        return lines
    }

    var body: some View {
        let lines = lines
        VStack(alignment: .leading, spacing: 10) {
            head
            if !lines.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                        Text("! \(line.text)")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(line.level.color)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if let snapshot = entry.snapshot {
                ForEach(entry.capacityVolumes, id: \.id) { volume in
                    FillBar(
                        name: volume.name,
                        percent: volume.usedPercent,
                        usedBytes: volume.usedBytes,
                        totalBytes: volume.totalBytes,
                        forecast: nil
                    )
                }
                MetricsRow(
                    diskTempC: entry.hottestDiskC,
                    cpuPercent: snapshot.system.cpuPercent,
                    ramPercent: snapshot.system.memoryPercent,
                    upsChargePercent: snapshot.ups?.chargePercent,
                    upsOnBattery: snapshot.ups?.state == .onBattery,
                    lastBackup: entry.lastBackupAt,
                    now: now
                )
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.cardBackground, in: shape)
        .overlay(alignment: .leading) {
            // Edge in the colour of the state while there is something to read.
            if !lines.isEmpty {
                UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 16)
                    .fill(entry.statusLevel.color)
                    .frame(width: 4)
            }
        }
        .overlay(shape.strokeBorder(.separator, lineWidth: 0.5))
        .clipShape(shape)
        .contentShape(shape)
    }

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
    }

    private var head: some View {
        HStack(spacing: 8) {
            StatusDot(level: entry.statusLevel)
            Text(verbatim: entry.displayName)
                .font(.headline)
                .lineLimit(1)
            if let model = entry.displayModel {
                Text(verbatim: model)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            if let vendor = entry.vendorLabel {
                Text(verbatim: vendor.uppercased())
                    .font(.caption2.weight(.bold))
                    .tracking(0.6)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(.separator))
                    .fixedSize()
            }
        }
    }
}

extension Color {
    /// Background of a card on the grouped background of the overview.
    static var cardBackground: Color {
        #if os(iOS)
        Color(uiColor: .secondarySystemGroupedBackground)
        #else
        Color(nsColor: .controlBackgroundColor)
        #endif
    }

    /// Background of the overview behind the cards.
    static var overviewBackground: Color {
        #if os(iOS)
        Color(uiColor: .systemGroupedBackground)
        #else
        Color(nsColor: .windowBackgroundColor)
        #endif
    }
}
