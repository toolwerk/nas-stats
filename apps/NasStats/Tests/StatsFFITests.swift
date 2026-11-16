import Foundation
import Testing

/// Calls the Rust core over the generated bindings and decodes every record
/// of the demo snapshot field by field (#12).
struct StatsFFITests {
    @Test func demoSnapshotDecodesEveryRecord() throws {
        let snapshot = demoSnapshot()
        #expect(snapshot.demo)
        #expect(snapshot.observedAt > 0)
        #expect(snapshot.worstHealth == .warning)

        let nas = snapshot.nas
        #expect(nas.id == "demo-keller")
        #expect(nas.provider == .qnap)
        #expect(nas.name == "Keller")
        #expect(nas.baseUrl.hasPrefix("https://"))
        #expect(nas.model == "TS-464")
        #expect(nas.firmware == "QTS 5.2.4")
        #expect(nas.hostname == "keller")

        let system = snapshot.system
        #expect(system.uptimeSeconds == 12 * 86_400 + 3 * 3_600)
        #expect(system.cpuPercent == 18.5)
        #expect(system.memoryPercent == 42.0)
        #expect(system.memoryTotalBytes == 17_179_869_184)
        #expect(system.temperatureC == 41.0)
        #expect(system.cpuTemperatureC == 56.0)
        #expect(system.updateAvailable == true)
        #expect(system.updateVersion == "QTS 5.2.5")

        let ups = try #require(snapshot.ups)
        #expect(ups.state == .online)
        #expect(ups.chargePercent == 100.0)
        #expect(ups.runtimeSeconds == 1_800)

        #expect(snapshot.volumes.count == 2)
        let pool = try #require(snapshot.volumes.first { $0.kind == .pool })
        let poolScrub = try #require(pool.scrub)
        #expect(poolScrub.state == .running)
        #expect(poolScrub.progressPercent == 42.0)
        let data = try #require(snapshot.volumes.first { $0.name == "DataVol1" })
        #expect(data.id == "vol-1")
        #expect(data.kind == .volume)
        #expect(data.poolId == pool.id)
        #expect(data.usedBytes == 13_936_000_000_000)
        #expect(data.totalBytes == 16_000_000_000_000)
        let used = try #require(data.usedPercent)
        #expect(abs(used - 87.1) < 1e-9)
        #expect(data.health == .warning)
        #expect(data.statusText == "Fast voll")
        #expect(data.raid == "RAID 5")
        let scrub = try #require(data.scrub)
        #expect(scrub.state == .finished)
        #expect(scrub.lastFinishedAt != nil)
        #expect(scrub.errors == 0)
        #expect(scrub.progressPercent == 100.0)

        #expect(snapshot.disks.count == 4)
        let disk = try #require(snapshot.disks.first { $0.id == "disk-3" })
        #expect(disk.name == "Platte 3")
        #expect(disk.model == "Demo HDD 8TB")
        #expect(disk.serialHash?.count == 64)
        #expect(disk.temperatureC == 52.0)
        #expect(disk.health == .warning)
        #expect(disk.smartText == "Warnung")
        #expect(disk.powerOnHours == 18_300)
        #expect(disk.spinning == true)

        let fan = try #require(snapshot.fans.first)
        #expect(fan.name == "Lüfter 1")
        #expect(fan.rpm == 1_020)
        #expect(fan.health == .ok)

        let backup = try #require(snapshot.backups.first)
        #expect(backup.id == "hbs-1")
        #expect(backup.name == "Nachtsicherung")
        #expect(backup.kind == .hbs3)
        #expect(backup.lastRun != nil)
        #expect(backup.lastResult == .ok)
        #expect(backup.nextRun != nil)

        let container = try #require(snapshot.containers.first)
        #expect(container.name == "demo-app")
        #expect(container.running)

        let alert = try #require(snapshot.alerts.first)
        #expect(alert.level == .warning)
        #expect(alert.text.contains("DataVol1"))
        #expect(alert.since != nil)

        let interface = try #require(snapshot.interfaces.first)
        #expect(interface.name == "Adapter 1")
        #expect(interface.rxBytesPerSec == 1_250_000.0)
        #expect(interface.txBytesPerSec == 310_000.0)
    }

    @Test func demoSnapshotsAreTheThreeDemoNas() {
        // 14 November 2026, 14:05 UTC.
        let now: Int64 = 1_794_665_100
        let snapshots = demoSnapshots(now: now, thresholds: standardThresholds())
        #expect(snapshots.map(\.nas.id) == ["demo-keller", "demo-buero", "demo-homelab"])
        #expect(snapshots.allSatisfy { $0.demo && $0.observedAt == now })
        #expect(snapshots == demoSnapshots(now: now, thresholds: standardThresholds()))
        #expect(snapshots[0].findings.map(\.text) == ["Platte 3 bei 52 °C", "DataVol1 bei 87 %"])
        #expect(snapshots[1].findings.isEmpty)
        #expect(snapshots[2].findings.isEmpty)
    }

    @Test func standardThresholdsAreThoseOfTheSettings() {
        #expect(standardThresholds() == AlertThresholds.standard.core)
    }

    @Test func demoSnapshotCarriesFindings() {
        let findings = demoSnapshot().findings
        #expect(!findings.isEmpty)
        #expect(findings.allSatisfy { $0.level != .ok })
    }

    @Test func newCoreKnowsNoNas() {
        #expect(NasCore().states().isEmpty)
    }

    @Test func refreshOfAnUnknownNasThrows() async {
        let core = NasCore()
        await #expect(throws: FfiError.UnknownNas(id: "nope")) {
            _ = try await core.refresh(nasId: "nope")
        }
    }

    @Test func addedNasHasNoStateYet() throws {
        let core = NasCore()
        let config = NasConfig(
            id: "nas-1",
            provider: .qnap,
            baseUrl: "https://nas.demo.invalid",
            fingerprint: String(repeating: "AB", count: 32)
        )
        try core.addNas(config: config, user: "monitor", password: "demo")
        let state = try #require(core.states().first)
        #expect(state.nasId == "nas-1")
        #expect(state.snapshot == nil)
        #expect(state.problem == nil)
        #expect(state.lastAttemptAt == nil)
        #expect(!state.blocked)
    }

    /// Port 1 on the loopback refuses the connection at once: no DNS, no
    /// outside network, and the problem record comes back.
    @Test func failedQueryReturnsTheProblem() async throws {
        let core = NasCore()
        let config = NasConfig(
            id: "nas-1",
            provider: .qnap,
            baseUrl: "https://127.0.0.1:1",
            fingerprint: String(repeating: "AB", count: 32)
        )
        try core.addNas(config: config, user: "monitor", password: "demo")
        let state = try await core.refresh(nasId: "nas-1")
        #expect(state.snapshot == nil)
        #expect(state.lastAttemptAt != nil)
        #expect(!state.blocked)
        let problem = try #require(state.problem)
        #expect(problem.kind == .unreachable)
        #expect(!problem.message.isEmpty)
        #expect(!problem.stopsPolling)
        #expect(problem.oldFingerprint == nil)
        #expect(problem.newFingerprint == nil)
        #expect(problem.httpStatus == nil)
    }
}
