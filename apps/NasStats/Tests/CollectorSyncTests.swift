import Foundation
import Synchronization
import Testing

/// Answers of the collector for `CollectorSync`, recorded per call.
final class FakeCollector: Sendable {
    let list = Mutex<[CollectorNas]>([])
    /// `listNas` throws this while set.
    let failure = Mutex<FfiError?>(nil)
    let calls = Mutex(0)

    var listNas: CollectorSync.ListNas {
        { [self] _, _, _ in
            calls.withLock { $0 += 1 }
            if let failure = failure.withLock({ $0 }) {
                throw failure
            }
            return list.withLock { $0 }
        }
    }
}

@MainActor
struct CollectorSyncTests {
    let defaults: UserDefaults
    let clock = TestClock()
    let fake = FakeCollector()

    static let fingerprint = String(repeating: "AB", count: 32)

    init() throws {
        defaults = try #require(UserDefaults(suiteName: "nasstats-collector-tests-\(UUID().uuidString)"))
    }

    static func collector(
        assignments: [String: UUID] = [:], ignored: Set<String> = []
    ) -> StoredCollector {
        StoredCollector(
            baseUrl: "https://collector.demo.invalid:8743",
            fingerprint: "CD:CD",
            token: "schluessel",
            assignments: assignments,
            ignored: ignored,
            syncsViaICloud: false
        )
    }

    static func nas(_ id: String, fingerprint: String = CollectorSyncTests.fingerprint) -> CollectorNas {
        CollectorNas(
            id: id, provider: .qnap, name: "nas-\(id)", model: "TS-464",
            baseUrl: "https://nas.demo.invalid", fingerprint: fingerprint,
            lastSeen: nil, blocked: false, error: nil
        )
    }

    /// An account of the app with `fingerprint`, registered with `core`.
    static func account(_ name: String, fingerprint: String = CollectorSyncTests.fingerprint) -> StoredAccount {
        var stored = NasStoreTests.account(name)
        stored.account.fingerprint = fingerprint
        return stored
    }

    func sync(
        _ store: MemoryCollectorStore, accounts: [StoredAccount], core: FakeCore = FakeCore()
    ) -> CollectorSync {
        CollectorSync(
            store: store,
            accounts: NasAccounts(keychain: MemoryAccountStore(accounts), core: core),
            defaults: defaults,
            clock: { [clock] in clock.now },
            listNas: fake.listNas
        )
    }

    // MARK: - Assignment

    @Test func theNasWithTheSameCertificateIsAssignedAutomatically() async throws {
        let a = Self.account("Keller", fingerprint: "ab:" + String(repeating: "ab", count: 31))
        let b = Self.account("Büro", fingerprint: String(repeating: "EF", count: 32))
        let core = FakeCore()
        fake.list.withLock { $0 = [Self.nas("1")] }
        let sync = sync(MemoryCollectorStore(Self.collector()), accounts: [a, b], core: core)

        await sync.sync()

        #expect(sync.assignments["1"] == CollectorSync.Assignment(appNasId: a.account.id, automatic: true))
        #expect(core.calls.imports == [
            FakeCore.CollectorImportCall(collectorNasId: "1", nasId: a.account.id.uuidString, since: nil),
        ])
    }

    @Test func aChoiceByHandWinsAndNotTakingOverStopsTheAutomatic() async throws {
        let a = Self.account("Keller")
        let b = Self.account("Büro", fingerprint: String(repeating: "EF", count: 32))
        let core = FakeCore()
        fake.list.withLock { $0 = [Self.nas("1"), Self.nas("2")] }
        let store = MemoryCollectorStore(Self.collector(assignments: ["1": b.account.id], ignored: ["2"]))
        let sync = sync(store, accounts: [a, b], core: core)

        await sync.sync()

        // NAS 2 would match Keller by its certificate, but the user said no.
        #expect(sync.assignments == ["1": CollectorSync.Assignment(appNasId: b.account.id, automatic: false)])
        #expect(core.calls.imports.map(\.nasId) == [b.account.id.uuidString])
    }

    @Test func assignStoresTheChoiceAndNilIgnoresTheNas() async throws {
        let a = Self.account("Keller")
        fake.list.withLock { $0 = [Self.nas("1")] }
        let store = MemoryCollectorStore(Self.collector())
        let sync = sync(store, accounts: [a])
        await sync.sync()

        try sync.assign("1", to: nil)
        #expect(store.stored?.ignored == ["1"])
        #expect(sync.assignments.isEmpty)

        try sync.assign("1", to: a.account.id)
        #expect(store.stored?.ignored == [])
        #expect(store.stored?.assignments == ["1": a.account.id])
        #expect(sync.assignments["1"]?.automatic == false)
    }

    // MARK: - Progress

    @Test func theNextImportStartsAtTheLastUntilAndAfterAChangeAgainAtTheStart() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        core.importResult.withLock { $0 = CollectorImport(until: 5_000, samples: 3, hourly: 0, daily: 0, backupRuns: 0) }
        fake.list.withLock { $0 = [Self.nas("1"), Self.nas("2", fingerprint: "00")] }
        let store = MemoryCollectorStore(Self.collector())
        let sync = sync(store, accounts: [a], core: core)
        let id = a.account.id.uuidString

        let imported = await sync.sync()
        #expect(imported == [id])
        await sync.sync()
        // Now the NAS of the app takes over another NAS of the collector.
        try sync.assign("2", to: a.account.id)
        await sync.sync()

        #expect(core.calls.imports == [
            FakeCore.CollectorImportCall(collectorNasId: "1", nasId: id, since: nil),
            FakeCore.CollectorImportCall(collectorNasId: "1", nasId: id, since: 5_000),
            FakeCore.CollectorImportCall(collectorNasId: "2", nasId: id, since: nil),
        ])
        // The progress lasts across launches.
        let again = self.sync(store, accounts: [a], core: core)
        #expect(again.state.progress[id] == CollectorSync.Progress(collectorNasId: "2", until: 5_000))
    }

    @Test func aNasNotRegisteredWithTheCoreIsNotImported() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        core.refuseAdd.withLock { $0 = true }
        fake.list.withLock { $0 = [Self.nas("1")] }
        let sync = sync(MemoryCollectorStore(Self.collector()), accounts: [a], core: core)

        await sync.sync()

        #expect(core.calls.imports.isEmpty)
    }

    @Test func aFailedImportOfOneNasDoesNotStopTheOthers() async throws {
        let a = Self.account("Keller")
        let b = Self.account("Büro", fingerprint: "EF")
        let core = FakeCore()
        core.failImport.withLock { $0 = ["1": .Http(status: 404)] }
        core.importResult.withLock { $0 = CollectorImport(until: 9, samples: 0, hourly: 1, daily: 0, backupRuns: 0) }
        fake.list.withLock { $0 = [Self.nas("1"), Self.nas("2", fingerprint: "EF")] }
        let sync = sync(MemoryCollectorStore(Self.collector()), accounts: [a, b], core: core)

        let imported = await sync.sync()

        #expect(imported == [b.account.id.uuidString])
        #expect(sync.nasErrors == ["1": "Nicht mehr im Container"])
        #expect(sync.error == nil)
    }

    // MARK: - When

    @Test func syncIfDueWaitsForTheInterval() async throws {
        let a = Self.account("Keller")
        fake.list.withLock { $0 = [Self.nas("1")] }
        let sync = sync(MemoryCollectorStore(Self.collector()), accounts: [a])

        await sync.syncIfDue(interval: 300)
        clock.advance(minutes: 4)
        await sync.syncIfDue(interval: 300)
        #expect(fake.calls.withLock { $0 } == 1)

        clock.advance(minutes: 1)
        await sync.syncIfDue(interval: 300)
        #expect(fake.calls.withLock { $0 } == 2)
    }

    @Test func withoutPairingNothingIsAsked() async throws {
        let sync = sync(MemoryCollectorStore(), accounts: [Self.account("Keller")])

        await sync.syncIfDue(interval: 300)

        #expect(fake.calls.withLock { $0 } == 0)
        #expect(sync.paired == nil)
    }

    @Test func aRefusedKeyStopsTheAutomaticSync() async throws {
        fake.failure.withLock { $0 = .LoginFailed }
        let sync = sync(MemoryCollectorStore(Self.collector()), accounts: [Self.account("Keller")])

        await sync.syncIfDue(interval: 300)
        clock.advance(minutes: 60)
        await sync.syncIfDue(interval: 300)

        #expect(fake.calls.withLock { $0 } == 1)
        #expect(sync.error == "Schlüssel abgelehnt – neu koppeln")
        #expect(sync.state.stopped == "Schlüssel abgelehnt – neu koppeln")
    }

    @Test func anUnreachableCollectorIsAskedAgainAfterTheInterval() async throws {
        fake.failure.withLock { $0 = .Unreachable(detail: "weg") }
        let sync = sync(MemoryCollectorStore(Self.collector()), accounts: [Self.account("Keller")])

        await sync.syncIfDue(interval: 300)
        #expect(sync.error == "Container nicht erreichbar")
        #expect(sync.state.stopped == nil)

        fake.failure.withLock { $0 = nil }
        clock.advance(minutes: 5)
        await sync.syncIfDue(interval: 300)

        #expect(fake.calls.withLock { $0 } == 2)
        #expect(sync.error == nil)
        #expect(sync.state.lastSuccessAt == clock.now)
    }

    // MARK: - Pairing

    @Test func aFailedPairingStoresNothing() async throws {
        fake.failure.withLock { $0 = .LoginFailed }
        let store = MemoryCollectorStore()
        let sync = sync(store, accounts: [])
        let collector = Self.collector()

        await #expect(throws: FfiError.LoginFailed) {
            try await sync.pair(
                baseUrl: collector.baseUrl, fingerprint: collector.fingerprint,
                token: collector.token, syncsViaICloud: true
            )
        }

        #expect(store.stored == nil)
        #expect(sync.paired == nil)
    }

    @Test func pairingStoresTheCollectorAndRemoveDeletesIt() async throws {
        let store = MemoryCollectorStore()
        let sync = sync(store, accounts: [Self.account("Keller")])

        try await sync.pair(
            baseUrl: "https://collector.demo.invalid:8743", fingerprint: "CD:CD",
            token: "schluessel", syncsViaICloud: true
        )
        #expect(store.stored == StoredCollector(
            baseUrl: "https://collector.demo.invalid:8743", fingerprint: "CD:CD",
            token: "schluessel", syncsViaICloud: true
        ))
        await sync.sync()
        #expect(defaults.data(forKey: CollectorSync.stateKey) != nil)

        try sync.remove()

        #expect(store.stored == nil)
        #expect(sync.paired == nil)
        #expect(defaults.data(forKey: CollectorSync.stateKey) == nil)
    }

    @Test func pairingAgainWithTheSameAddressKeepsTheChoices() async throws {
        let a = Self.account("Keller")
        let store = MemoryCollectorStore(Self.collector(assignments: ["1": a.account.id], ignored: ["2"]))
        let sync = sync(store, accounts: [a])

        try await sync.pair(
            baseUrl: "https://collector.demo.invalid:8743", fingerprint: "EE:EE",
            token: "neu", syncsViaICloud: false
        )

        #expect(store.stored?.assignments == ["1": a.account.id])
        #expect(store.stored?.ignored == ["2"])
        #expect(store.stored?.token == "neu")
    }

    @Test func iCloudSettingMovesTheEntry() async throws {
        let store = MemoryCollectorStore(Self.collector())
        let sync = sync(store, accounts: [])

        try sync.setSyncsViaICloud(true)

        #expect(store.stored?.syncsViaICloud == true)
    }

    // MARK: - Texts

    @Test func addressWithoutPortGetsThePortOfTheCollector() throws {
        #expect(try CollectorAddress.normalize("192.168.178.20").get() == "https://192.168.178.20:8743")
        #expect(try CollectorAddress.normalize("https://nas.local:9000").get() == "https://nas.local:9000")
        #expect(throws: NasAddressError.notHTTPS) { try CollectorAddress.normalize("http://nas").get() }
    }

    @Test func fingerprintsCompareWithoutColonsAndCase() {
        #expect(sameFingerprint("AB:CD:EF", "abcdef"))
        #expect(!sameFingerprint("AB:CD", "AB:CE"))
        #expect(!sameFingerprint("", ""))
    }
}
