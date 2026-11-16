import Foundation
import Synchronization
import Testing

/// Private database double: keeps the records by name; a fetch without a
/// token returns all of them, one with a token those written after it.
final class FakeCloudDatabase: CloudDatabase {
    struct State {
        var status: CloudAccountStatus = .available
        var zoneExists = false
        var records: [String: CloudRecord] = [:]
        /// Every record of every `save`, in order.
        var saved: [CloudRecord] = []
        var failSave = false
        var failDelete = false
        var zoneDeletes = 0
        var subscriptions = 0
        /// Token of every `changes` call.
        var fetchTokens: [Data?] = []
        /// `changes` with a token throws `tokenExpired` while set.
        var expireTokens = false
    }

    struct Refused: Error {}

    let state = Mutex(State())

    func accountStatus() async throws -> CloudAccountStatus {
        state.withLock { $0.status }
    }

    func ensureZone() async throws {
        state.withLock { $0.zoneExists = true }
    }

    func save(_ records: [CloudRecord]) async throws {
        try state.withLock { state in
            if state.failSave { throw Refused() }
            guard state.zoneExists else { throw CloudDatabaseError.zoneNotFound }
            for record in records {
                state.records[record.name] = record
            }
            state.saved += records
        }
    }

    func changes(since token: Data?) async throws -> CloudChanges {
        try state.withLock { state in
            state.fetchTokens.append(token)
            if state.expireTokens, token != nil { throw CloudDatabaseError.tokenExpired }
            guard state.zoneExists else { throw CloudDatabaseError.zoneNotFound }
            let after = token.flatMap { Int64(String(decoding: $0, as: UTF8.self)) } ?? .min
            let records = state.records.values
                .filter { $0.writtenAt > after }
                .sorted { $0.name < $1.name }
            let newest = state.records.values.map(\.writtenAt).max() ?? after
            return CloudChanges(records: records, token: Data(String(newest).utf8))
        }
    }

    func deleteZone() async throws {
        try state.withLock { state in
            if state.failDelete { throw Refused() }
            state.zoneExists = false
            state.records = [:]
            state.zoneDeletes += 1
        }
    }

    func subscribe() async throws {
        state.withLock { $0.subscriptions += 1 }
    }

    /// Puts a record in the zone as the Mac would.
    func put(_ record: CloudRecord) {
        state.withLock { state in
            state.zoneExists = true
            state.records[record.name] = record
        }
    }

    var current: State { state.withLock { $0 } }
}

/// What the `apply` of a fetch got, per fetch.
@MainActor
final class Taken {
    var lists: [[String]] = []
}

@MainActor
struct CloudTransferTests {
    let defaults: UserDefaults
    let clock = TestClock()

    init() throws {
        defaults = try #require(UserDefaults(suiteName: "nasstats-cloud-tests-\(UUID().uuidString)"))
        defaults.set(false, forKey: NasStore.sharesAccessViaICloudKey)
    }

    func transfer(_ database: FakeCloudDatabase, role: CloudTransfer.Role) -> CloudTransfer {
        CloudTransfer(role: role, database: database, defaults: defaults, clock: { [clock] in clock.now })
    }

    func store(
        _ accounts: [StoredAccount],
        core: FakeCore,
        cloud: CloudTransfer,
        notifier: EventNotifier? = nil
    ) -> NasStore {
        NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore(accounts), core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            notifier: notifier,
            cloud: cloud
        )
    }

    static func snapshot(_ id: String, observedAt: Int64) -> Snapshot {
        var snapshot = NasStoreTests.snapshot(id, health: .ok)
        snapshot.observedAt = observedAt
        return snapshot
    }

    static func state(_ id: String, observedAt: Int64) -> NasState {
        NasState(
            nasId: id,
            snapshot: snapshot(id, observedAt: observedAt),
            problem: nil,
            lastAttemptAt: observedAt,
            blocked: false
        )
    }

    // MARK: - Records

    @Test func recordsHaveTheirNamesAndSkipATooLargePayload() throws {
        let state = try #require(CloudRecord.state(
            nasId: "n1", observedAt: 7, payload: "{}", recent: "[]", writtenAt: 9
        ))
        #expect(state.name == "state-n1")
        #expect(state.kind.rawValue == "NasState")
        #expect(state.time == 7)
        #expect(state.payloadText == "{}")
        #expect(state.recentText == "[]")
        #expect(state.version == 1)
        let day = try #require(CloudRecord.day(nasId: "n1", day: 86_400, payload: "{}", writtenAt: 9))
        #expect(day.name == "day-n1-86400")
        #expect(day.kind.rawValue == "NasDay")
        #expect(day.recent == nil)

        let large = String(repeating: "x", count: CloudRecord.maxPayload + 1)
        #expect(CloudRecord.day(nasId: "n1", day: 0, payload: large, writtenAt: 9) == nil)
        #expect(CloudRecord.state(nasId: "n1", observedAt: 0, payload: large, recent: nil, writtenAt: 9) == nil)
    }

    // MARK: - Mac

    /// A core with one NAS that has a state, and its id.
    func macCore() async throws -> (FakeCore, String) {
        let a = NasStoreTests.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(id, observedAt: 500)])
        // Registers the NAS with the core.
        _ = NasAccounts(keychain: MemoryAccountStore([a]), core: core)
        _ = try await core.refresh(nasId: id)
        return (core, id)
    }

    @Test func theMacWritesStateAndEveryDayThenFromTheMarkOn() async throws {
        let (core, id) = try await macCore()
        core.daysPreset.withLock { $0 = [0, 86_400] }
        let database = FakeCloudDatabase()
        let cloud = transfer(database, role: .upload)

        await cloud.upload(nasId: id, observedAt: 500, core: core)
        let first = database.current.saved
        #expect(first.map(\.name).sorted() == ["day-\(id)-0", "day-\(id)-86400", "state-\(id)"])
        let state = try #require(first.first { $0.kind == .state })
        #expect(state.payloadText == FakeCore.saved(id))
        #expect(state.recentText == "recent:\(id)")
        #expect(state.time == 500)
        #expect(cloud.uploads == [id: 86_400])
        #expect(cloud.status == .transferred(Date(timeIntervalSince1970: TimeInterval(clock.now))))

        // The last day again, it may have grown, and the new one.
        core.daysPreset.withLock { $0 = [0, 86_400, 172_800] }
        clock.advance(minutes: 5)
        await cloud.upload(nasId: id, observedAt: 800, core: core)
        let second = database.current.saved.dropFirst(first.count)
        #expect(second.map(\.name).sorted() == ["day-\(id)-172800", "day-\(id)-86400", "state-\(id)"])
        #expect(cloud.uploads == [id: 172_800])
    }

    @Test func aFailedSaveKeepsTheMarkAndShowsTheError() async throws {
        let (core, id) = try await macCore()
        core.daysPreset.withLock { $0 = [0, 86_400] }
        let database = FakeCloudDatabase()
        let cloud = transfer(database, role: .upload)
        database.state.withLock { $0.failSave = true }
        await cloud.upload(nasId: id, observedAt: 500, core: core)
        #expect(cloud.uploads.isEmpty)
        guard case .failed = cloud.status else {
            Issue.record("no error shown: \(cloud.status)")
            return
        }

        // Without iCloud nothing is written.
        database.state.withLock {
            $0.failSave = false
            $0.status = .noAccount
        }
        await cloud.upload(nasId: id, observedAt: 500, core: core)
        #expect(cloud.status == .failed(CloudTransfer.notSignedIn))
        #expect(database.current.saved.isEmpty)
    }

    @Test func aZoneDeletedElsewhereSendsEverythingAgain() async throws {
        let (core, id) = try await macCore()
        core.daysPreset.withLock { $0 = [0, 86_400] }
        let database = FakeCloudDatabase()
        let cloud = transfer(database, role: .upload)
        await cloud.upload(nasId: id, observedAt: 500, core: core)
        try await database.deleteZone()

        await cloud.upload(nasId: id, observedAt: 600, core: core)
        #expect(cloud.uploads.isEmpty)
        await cloud.upload(nasId: id, observedAt: 700, core: core)
        #expect(database.current.records.keys.sorted() == ["day-\(id)-0", "day-\(id)-86400", "state-\(id)"])
        #expect(cloud.uploads == [id: 86_400])
    }

    @Test func bothSwitchesAreOffAndTransferNothing() async throws {
        let a = NasStoreTests.account("Keller")
        for role in [CloudTransfer.Role.upload, .download] {
            let core = FakeCore(states: [Self.state(a.account.id.uuidString, observedAt: 500)])
            let database = FakeCloudDatabase()
            database.put(CloudRecord.state(nasId: a.account.id.uuidString, observedAt: 900, payload: "mac", recent: nil, writtenAt: 1)!)
            let store = store([a], core: core, cloud: transfer(database, role: role))
            store.start()
            await store.refreshAll()
            await store.cloudWork?.value
            #expect(!store.transfersViaICloud)
            #expect(await store.fetchFromCloud() == false)
            #expect(database.current.saved.isEmpty)
            #expect(database.current.fetchTokens.isEmpty)
            #expect(core.calls.remoteStates.isEmpty)
        }
    }

    @Test func theMacWritesEveryNewStateWhileOn() async throws {
        let a = NasStoreTests.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(id, observedAt: 500)])
        let database = FakeCloudDatabase()
        let store = store([a], core: core, cloud: transfer(database, role: .upload))
        store.start()
        await store.refreshAll()

        try await store.setTransfersViaICloud(true)
        #expect(store.transfersViaICloud)
        #expect(database.current.saved.map(\.name) == ["state-\(id)"])

        // A new snapshot of the planner goes up, the same one again does not.
        core.plannerQueried(Self.state(id, observedAt: 800))
        try await waitFor { database.current.saved.count == 2 }
        #expect(database.current.saved.last?.time == 800)
        core.plannerQueried(Self.state(id, observedAt: 800))
        core.plannerQueried(Self.state(id, observedAt: 900))
        try await waitFor { database.current.saved.count == 3 }
        #expect(database.current.saved.map(\.time) == [500, 800, 900])
    }

    @Test func switchingOffOnTheMacDeletesTheDataAfterwards() async throws {
        let a = NasStoreTests.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(id, observedAt: 500)])
        let database = FakeCloudDatabase()
        let cloud = transfer(database, role: .upload)
        let store = store([a], core: core, cloud: cloud)
        store.start()
        await store.refreshAll()
        try await store.setTransfersViaICloud(true)

        // iCloud refuses: the switch stays on.
        database.state.withLock { $0.failDelete = true }
        await #expect(throws: (any Error).self) { try await store.setTransfersViaICloud(false) }
        #expect(store.transfersViaICloud)

        database.state.withLock { $0.failDelete = false }
        try await store.setTransfersViaICloud(false)
        #expect(!store.transfersViaICloud)
        #expect(database.current.zoneDeletes == 1)
        #expect(database.current.records.isEmpty)
        #expect(cloud.uploads.isEmpty)
        #expect(cloud.status == CloudTransfer.Status.none)
        #expect(!defaults.bool(forKey: NasStore.transfersViaICloudKey))
    }

    @Test func theDemoModeTransfersNothing() async throws {
        defaults.set(true, forKey: NasStore.demoModeKey)
        let a = NasStoreTests.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(id, observedAt: 500)])
        for role in [CloudTransfer.Role.upload, .download] {
            let database = FakeCloudDatabase()
            database.put(CloudRecord.state(nasId: id, observedAt: 900, payload: "mac", recent: nil, writtenAt: 1)!)
            let store = store([a], core: core, cloud: transfer(database, role: role))
            store.start()
            try await store.setTransfersViaICloud(true)
            await store.cloudWork?.value
            #expect(await store.fetchFromCloud() == false)
            #expect(database.current.saved.isEmpty)
            #expect(database.current.fetchTokens.isEmpty)
        }
        #expect(core.calls.remoteStates.isEmpty)
    }

    // MARK: - iPhone

    @Test func thePhoneTakesOverOnlyItsOwnNas() async throws {
        let a = NasStoreTests.account("Keller")
        let id = a.account.id.uuidString
        let other = UUID().uuidString
        let core = FakeCore(states: [Self.state(id, observedAt: 500)])
        core.makeRestorable("mac:\(id)", Self.snapshot(id, observedAt: 900))
        let database = FakeCloudDatabase()
        for nas in [id, other] {
            database.put(CloudRecord.state(nasId: nas, observedAt: 900, payload: "mac:\(nas)", recent: "recent:\(nas)", writtenAt: 1_000)!)
            database.put(CloudRecord.day(nasId: nas, day: 86_400, payload: "day2:\(nas)", writtenAt: 1_000)!)
            database.put(CloudRecord.day(nasId: nas, day: 0, payload: "day1:\(nas)", writtenAt: 1_000)!)
        }
        var newer = CloudRecord.day(nasId: id, day: 172_800, payload: "day3", writtenAt: 1_000)!
        newer.version = 2
        database.put(newer)
        let cloud = transfer(database, role: .download)
        let store = store([a], core: core, cloud: cloud)
        // The own query fails away from home.
        core.failRefresh.withLock { $0 = true }
        store.start()
        await store.refreshAll()
        #expect(store.entries.first?.queryError != nil)

        try await store.setTransfersViaICloud(true)
        #expect(core.calls.importedHistory == ["\(id)|day1:\(id)", "\(id)|day2:\(id)", "\(id)|recent:\(id)"])
        #expect(core.calls.remoteStates == ["\(id)|mac:\(id)"])
        let entry = try #require(store.entries.first)
        #expect(entry.snapshot?.observedAt == 900)
        #expect(entry.queryError == nil)
        #expect(cloud.status == .transferred(Date(timeIntervalSince1970: 1_000)))
        #expect(database.current.subscriptions == 1)

        // Nothing new: nothing is taken over again.
        #expect(await store.fetchFromCloud() == false)
        #expect(core.calls.remoteStates.count == 1)
    }

    @Test func anExpiredTokenFetchesEverythingOnceMore() async throws {
        let database = FakeCloudDatabase()
        database.put(CloudRecord.state(nasId: "n1", observedAt: 900, payload: "mac", recent: nil, writtenAt: 1_000)!)
        let cloud = transfer(database, role: .download)
        let taken = Taken()
        await cloud.fetch(knownIds: ["n1"]) { taken.lists.append($0.map(\.name)) }
        database.state.withLock { $0.expireTokens = true }
        await cloud.fetch(knownIds: ["n1"]) { taken.lists.append($0.map(\.name)) }
        let tokens = database.current.fetchTokens
        #expect(tokens.count == 3)
        #expect(tokens[0] == nil)
        #expect(tokens[1] != nil)
        #expect(tokens[2] == nil)
        #expect(taken.lists == [["state-n1"], ["state-n1"]])
    }

    @Test func aNewNasFetchesEverythingAgain() async throws {
        let database = FakeCloudDatabase()
        database.put(CloudRecord.state(nasId: "n1", observedAt: 900, payload: "mac", recent: nil, writtenAt: 1_000)!)
        database.put(CloudRecord.state(nasId: "n2", observedAt: 900, payload: "mac", recent: nil, writtenAt: 1_000)!)
        let cloud = transfer(database, role: .download)
        let taken = Taken()
        await cloud.fetch(knownIds: ["n1"]) { taken.lists.append($0.map(\.nasId)) }
        await cloud.fetch(knownIds: ["n1"]) { taken.lists.append($0.map(\.nasId)) }
        await cloud.fetch(knownIds: ["n1", "n2"]) { taken.lists.append($0.map(\.nasId)) }
        #expect(taken.lists == [["n1"], [], ["n1", "n2"]])
        #expect(database.current.fetchTokens.last == .some(nil))
    }

    @Test func noZoneMeansNoDataFromTheMacYet() async throws {
        let database = FakeCloudDatabase()
        let cloud = transfer(database, role: .download)
        let count = await cloud.fetch(knownIds: ["n1"]) { _ in }
        #expect(count == 0)
        #expect(cloud.status == .noData)
        database.state.withLock { $0.status = .noAccount }
        await cloud.fetch(knownIds: ["n1"]) { _ in }
        #expect(cloud.status == .failed(CloudTransfer.notSignedIn))
    }

    @Test func eventsOfTheMacNeverNotifyOnThePhone() async throws {
        let a = NasStoreTests.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(id, observedAt: 500)])
        core.makeRestorable("mac:\(id)", Self.snapshot(id, observedAt: 900))
        let open = NasEvent(
            nasId: id, kind: .diskTemperature, subject: "3", level: .warning,
            text: "Platte 3 bei 52 °C", startedAt: 1_000, endedAt: nil
        )
        core.presetEvents.withLock { $0[id] = [open] }
        let database = FakeCloudDatabase()
        database.put(CloudRecord.state(nasId: id, observedAt: 900, payload: "mac:\(id)", recent: nil, writtenAt: 1_000)!)
        let poster = FakePoster()
        let notifier = EventNotifier(poster: poster, defaults: defaults)
        await notifier.refreshAuthorization()
        let store = store([a], core: core, cloud: transfer(database, role: .download), notifier: notifier)
        try await store.setTransfersViaICloud(true)
        store.start()
        await store.refreshAll()
        #expect(core.calls.refreshed == [id])
        #expect(poster.posts.isEmpty)

        // A rise to critical stays silent too: the Mac notifies it.
        var critical = open
        critical.level = .critical
        core.presetEvents.withLock { $0[id] = [critical] }
        clock.advance(minutes: 5)
        await store.refreshAll()
        #expect(poster.posts.isEmpty)
    }

    /// Waits until `condition` holds, at most a second.
    private func waitFor(_ condition: () -> Bool) async throws {
        for _ in 0..<100 where !condition() {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(condition())
    }
}
