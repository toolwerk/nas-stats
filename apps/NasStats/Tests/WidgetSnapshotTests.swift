import Foundation
import Testing

@MainActor
struct WidgetSnapshotTests {
    let defaults: UserDefaults
    let folder: URL

    init() throws {
        defaults = try #require(UserDefaults(suiteName: "nasstats-widget-tests-\(UUID().uuidString)"))
        folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("nasstats-widget-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    }

    var fileURL: URL { folder.appendingPathComponent(WidgetSnapshot.fileName) }

    func demoStore() -> NasStore {
        let store = NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore(), core: FakeCore()),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: nil
        )
        store.demoMode = true
        return store
    }

    func demoSnapshot(_ store: NasStore, now: Date = .now) -> WidgetSnapshot {
        WidgetSnapshot(
            entries: store.entries, demo: store.demoMode,
            queryIntervalMinutes: store.queryIntervalMinutes,
            thresholds: store.thresholds, now: now
        )
    }

    /// One NAS seen `minutes` ago, outside demo mode.
    static func live(observedMinutesAgo minutes: Double?, interval: Int, now: Date) -> (WidgetSnapshot, WidgetNas) {
        let nas = WidgetNas(
            id: "a", name: "Keller", model: nil, vendor: "QNAP", status: .ok,
            firstFinding: nil, firstFindingLevel: nil, findingCount: 0, problemLine: nil,
            volumes: [], hottestDiskC: nil, cpuPercent: nil, memoryPercent: nil,
            lastBackupAt: nil,
            observedAt: minutes.map { now.addingTimeInterval(-$0 * 60) },
            fullestVolumeHistory: []
        )
        let snapshot = WidgetSnapshot(
            writtenAt: now, demo: false,
            staleAfterSeconds: WidgetSnapshot.staleAfter(intervalMinutes: interval),
            thresholds: .standard, nas: [nas]
        )
        return (snapshot, nas)
    }

    @Test func theDemoEntriesGiveThreeNasWorstFirst() throws {
        let store = demoStore()
        let snapshot = demoSnapshot(store)
        #expect(snapshot.schemaVersion == WidgetSnapshot.currentVersion)
        #expect(snapshot.demo)
        #expect(snapshot.nas.count == 3)
        #expect(snapshot.nas.map(\.id) == store.entries.map(\.id))
        let keller = try #require(snapshot.nas.first)
        #expect(keller.id == "demo-keller")
        let entry = try #require(store.entries.first)
        #expect(keller.firstFinding == entry.findings.first?.text)
        #expect(keller.firstFinding != nil)
        #expect(keller.firstFindingLevel == .warning || keller.firstFindingLevel == .critical)
        #expect(keller.findingCount == entry.findings.count)
        #expect(keller.status == entry.statusLevel)
        #expect(keller.name == entry.displayName)
        #expect(keller.vendor == "QNAP")
        #expect(keller.volumes.map(\.id) == entry.capacityVolumes.map(\.id))
        #expect(keller.hottestDiskC == entry.hottestDiskC)
        #expect(keller.cpuPercent == entry.snapshot?.system.cpuPercent)
        #expect(keller.memoryPercent == entry.snapshot?.system.memoryPercent)
        #expect(keller.lastBackupAt == entry.lastBackupAt)
        #expect(keller.observedAt != nil)
        // Forecast and history follow with #67.
        #expect(snapshot.nas.allSatisfy { $0.fullestVolumeHistory.isEmpty })
        #expect(snapshot.nas.flatMap(\.volumes).allSatisfy { $0.forecast == nil })
    }

    @Test func theAgeOfAnOldStateFollowsTheInterval() {
        #expect(WidgetSnapshot.staleAfter(intervalMinutes: 5) == 1800)
        #expect(WidgetSnapshot.staleAfter(intervalMinutes: 15) == 1800)
        #expect(WidgetSnapshot.staleAfter(intervalMinutes: 30) == 2100)
    }

    @Test func aStateTurnsOldAfterThirtyOrIntervalPlusFiveMinutes() {
        let now = Date(timeIntervalSince1970: 1_794_665_100)
        let (five, fiveNas) = Self.live(observedMinutesAgo: 31, interval: 5, now: now)
        #expect(five.isStale(fiveNas, now: now))
        let (fresh, freshNas) = Self.live(observedMinutesAgo: 29, interval: 5, now: now)
        #expect(!fresh.isStale(freshNas, now: now))
        let (thirty, thirtyNas) = Self.live(observedMinutesAgo: 31, interval: 30, now: now)
        #expect(!thirty.isStale(thirtyNas, now: now))
        let (late, lateNas) = Self.live(observedMinutesAgo: 36, interval: 30, now: now)
        #expect(late.isStale(lateNas, now: now))
        // The timeline turns grey at the first second past the limit.
        let date = fresh.staleDate(freshNas)
        #expect(date == freshNas.observedAt?.addingTimeInterval(1801))
        if let date {
            #expect(!fresh.isStale(freshNas, now: date.addingTimeInterval(-1)))
            #expect(fresh.isStale(freshNas, now: date))
        }
    }

    @Test func demoAndMissingStatesNeverTurnOld() {
        let now = Date(timeIntervalSince1970: 1_794_665_100)
        var (demo, demoNas) = Self.live(observedMinutesAgo: 120, interval: 5, now: now)
        demo.demo = true
        #expect(!demo.isStale(demoNas, now: now))
        #expect(demo.staleDate(demoNas) == nil)
        let (none, noneNas) = Self.live(observedMinutesAgo: nil, interval: 5, now: now)
        #expect(!none.isStale(noneNas, now: now))
        #expect(none.staleDate(noneNas) == nil)
    }

    @Test func theWriterWritesOnlyChangedContent() throws {
        var reloads = 0
        let writer = WidgetSnapshotWriter(fileURL: fileURL) { reloads += 1 }
        let store = demoStore()
        let first = demoSnapshot(store, now: Date(timeIntervalSince1970: 1_794_665_100))
        writer.update(first)
        #expect(reloads == 1)
        #expect(WidgetSnapshot.load(from: fileURL) == first)

        // Same content, written later: no redraw, the file stays.
        var later = first
        later.writtenAt = first.writtenAt.addingTimeInterval(300)
        writer.update(later)
        #expect(reloads == 1)
        #expect(WidgetSnapshot.load(from: fileURL)?.writtenAt == first.writtenAt)

        var changed = later
        changed.nas[0].name = "Dachboden"
        writer.update(changed)
        #expect(reloads == 2)
        #expect(WidgetSnapshot.load(from: fileURL) == changed)
    }

    @Test func withoutAFileTheWriterDoesNotRedraw() {
        var reloads = 0
        let writer = WidgetSnapshotWriter(fileURL: nil) { reloads += 1 }
        writer.update(demoSnapshot(demoStore()))
        #expect(reloads == 0)
        // A folder that does not exist fails the write.
        let broken = WidgetSnapshotWriter(
            fileURL: folder.appendingPathComponent("fehlt/\(WidgetSnapshot.fileName)")
        ) { reloads += 1 }
        broken.update(demoSnapshot(demoStore()))
        #expect(reloads == 0)
    }

    @Test func loadRefusesMissingBrokenAndNewerFiles() throws {
        #expect(WidgetSnapshot.load(from: nil) == nil)
        #expect(WidgetSnapshot.load(from: fileURL) == nil)
        try Data("kein JSON".utf8).write(to: fileURL)
        #expect(WidgetSnapshot.load(from: fileURL) == nil)
        var newer = demoSnapshot(demoStore())
        newer.schemaVersion = 2
        try WidgetSnapshot.encoder.encode(newer).write(to: fileURL)
        #expect(WidgetSnapshot.load(from: fileURL) == nil)
    }

    @Test func changingTheIntervalReportsTheEntries() {
        let store = demoStore()
        var reported = 0
        store.onUpdate = { _ in reported += 1 }
        store.queryIntervalMinutes = 30
        #expect(reported == 1)
        // The same value reports nothing.
        store.queryIntervalMinutes = 30
        #expect(reported == 1)
    }
}

@MainActor
struct WidgetSnapshotCarryOverTests {
    let now = Date(timeIntervalSince1970: 1_794_665_100)

    func nas(_ id: String, status: StatusLevel, observedMinutesAgo: Double?, finding: String? = nil) -> WidgetNas {
        var (_, nas) = WidgetSnapshotTests.live(observedMinutesAgo: observedMinutesAgo, interval: 5, now: now)
        nas.id = id
        nas.name = id
        nas.status = status
        nas.firstFinding = finding
        return nas
    }

    func snapshot(_ nas: [WidgetNas], demo: Bool = false) -> WidgetSnapshot {
        WidgetSnapshot(writtenAt: now, demo: demo, staleAfterSeconds: 1800, thresholds: .standard, nas: nas)
    }

    @Test func aNasWithoutStateKeepsItsLastOne() throws {
        let previous = snapshot([
            nas("a", status: .warning, observedMinutesAgo: 40, finding: "Platte 3 bei 52 °C"),
            nas("b", status: .ok, observedMinutesAgo: 3),
        ])
        // After a launch: b answered again, a waits for its interval.
        var blocked = nas("a", status: .critical, observedMinutesAgo: nil)
        blocked.problemLine = "Anmeldung abgelehnt"
        let merged = snapshot([blocked, nas("b", status: .ok, observedMinutesAgo: 0)])
            .carryingOver(from: previous)
        let a = try #require(merged.nas.first)
        #expect(a.id == "a")
        #expect(a.status == .critical)
        #expect(a.firstFinding == "Platte 3 bei 52 °C")
        #expect(a.problemLine == "Anmeldung abgelehnt")
        #expect(a.observedAt == previous.nas[0].observedAt)
        // Its old state shows its age.
        #expect(merged.isStale(a, now: now))
        #expect(merged.nas[1].observedAt == now)
    }

    @Test func anUnknownStateKeepsTheLastLevelAndTheOrder() {
        let previous = snapshot([nas("a", status: .warning, observedMinutesAgo: 3)])
        let merged = snapshot([
            nas("b", status: .ok, observedMinutesAgo: 0),
            nas("a", status: .unknown, observedMinutesAgo: nil),
        ]).carryingOver(from: previous)
        #expect(merged.nas.map(\.id) == ["a", "b"])
        #expect(merged.nas[0].status == .warning)
    }

    @Test func demoAndLiveDataNeverMix() {
        let previous = snapshot([nas("a", status: .warning, observedMinutesAgo: 3)], demo: true)
        let fresh = snapshot([nas("a", status: .unknown, observedMinutesAgo: nil)])
        #expect(fresh.carryingOver(from: previous) == fresh)
        #expect(fresh.carryingOver(from: nil) == fresh)
    }

    @Test func theWriterKeepsTheStateOfAnEarlierLaunch() throws {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("nasstats-widget-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent(WidgetSnapshot.fileName)
        let earlier = snapshot([nas("a", status: .ok, observedMinutesAgo: 3)])
        WidgetSnapshotWriter(fileURL: url) {}.update(earlier)

        let writer = WidgetSnapshotWriter(fileURL: url) {}
        writer.update(snapshot([nas("a", status: .unknown, observedMinutesAgo: nil)]))
        let read = try #require(WidgetSnapshot.load(from: url))
        #expect(read.nas.first?.observedAt == earlier.nas.first?.observedAt)
        #expect(read.nas.first?.status == .ok)
    }
}
