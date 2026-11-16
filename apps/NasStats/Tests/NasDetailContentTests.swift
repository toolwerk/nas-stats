import Foundation
import Testing

/// Sections of the detail screen from a snapshot (#18).
struct NasDetailContentTests {
    /// 14 November 2026, 14:05 UTC.
    private let now: Int64 = 1_794_665_100

    private func demo(_ id: String) -> Snapshot {
        demoSnapshots(now: now).first { $0.nas.id == id }!
    }

    @Test func kellerHasEverySection() {
        let content = NasDetailContent(snapshot: demo("demo-keller"))
        #expect(content.volumes.map(\.name) == ["DataVol1"])
        #expect(content.pools.map(\.name) == ["Speicherpool 1"])
        #expect(demo("demo-keller").disks.count == 4)
        guard case let .tasks(tasks) = content.backups else {
            Issue.record("Keller has backup tasks")
            return
        }
        #expect(tasks.map(\.name) == ["Offsite B2", "Snapshots DataVol1"])
        #expect(content.containerSummary == "6 / 7 laufen")
        #expect(content.containers.first?.name == "Watchtower")
        #expect(content.containers.dropFirst().map(\.name) == [
            "Immich", "Jellyfin", "Mosquitto", "Paperless", "Uptime Kuma", "Vaultwarden",
        ])
    }

    @Test func qnapWithoutBackupsShowsTheOpenInterface() {
        var keller = demo("demo-keller")
        keller.backups = []
        #expect(NasDetailContent(snapshot: keller).backups == .interfaceOpen)
    }

    @Test func otherVendorWithoutBackupsHidesTheSection() {
        var buero = demo("demo-buero")
        buero.backups = []
        #expect(NasDetailContent(snapshot: buero).backups == .hidden)
    }

    @Test func emptyListsLeaveOutTheirParts() {
        var keller = demo("demo-keller")
        keller.volumes = []
        keller.containers = []
        let content = NasDetailContent(snapshot: keller)
        #expect(content.volumes.isEmpty)
        #expect(content.pools.isEmpty)
        #expect(content.containers.isEmpty)
        #expect(content.containerSummary == nil)
    }

    @Test func labels() {
        #expect(partLabel(.ok) == "Gut")
        #expect(partLabel(.warning) == "Warnung")
        #expect(partLabel(.critical) == "Kritisch")
        #expect(partLabel(nil) == "n/a")
        #expect(partLevel(.warning) == .warning)
        #expect(partLevel(nil) == .unknown)
        #expect(backupKindLabel(.hbs3) == "Hybrid Backup Sync")
        #expect(backupKindLabel(.parity) == "Paritätsprüfung")
    }

    /// Büro crosses no threshold, so its health is that of the snapshot.
    private func entry(problem kind: ProblemKind?, stops: Bool = false) -> NasEntry {
        var snapshot = demo("demo-buero")
        snapshot.worstHealth = .ok
        var entry = NasEntry(id: "a", baseUrl: "https://a.invalid", snapshot: snapshot)
        entry.problem = kind.map {
            NasProblem(
                kind: $0, message: "Fehler", stopsPolling: stops,
                oldFingerprint: nil, newFingerprint: nil, httpStatus: nil)
        }
        return entry
    }

    @Test func statusLevel() {
        #expect(entry(problem: nil).statusLevel == .ok)
        #expect(entry(problem: .timeout).statusLevel == .offline)
        #expect(entry(problem: .unreachable).statusLevel == .offline)
        // A critical NAS stays critical when it stops answering (#17).
        var blocked = entry(problem: .unreachable)
        blocked.blocked = true
        #expect(blocked.statusLevel == .critical)
        // A refused login is no silence: as health, here critical.
        #expect(entry(problem: .loginFailed, stops: true).statusLevel == .critical)
        // An error without a block lifts to warning.
        #expect(entry(problem: .http).statusLevel == .warning)
        #expect(NasEntry(id: "b", baseUrl: "https://b.invalid").statusLevel == .unknown)
    }
}
