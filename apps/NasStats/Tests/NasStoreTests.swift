import Foundation
import Synchronization
import Testing

/// Core double: answers with preset states and records every call.
final class FakeCore: NasCoreProtocol {
    struct Calls {
        var added: [String] = []
        /// Fingerprint of every `addNas` call.
        var addedFingerprints: [String] = []
        var removed: [String] = []
        var refreshed: [String] = []
        var updated: [String] = []
        /// User of every `updateCredentials` call.
        var updatedUsers: [String] = []
        /// Password of every `updateCredentials` call.
        var updatedPasswords: [String] = []
        /// NAS id of every `restoreState` call.
        var restored: [String] = []
        /// Thresholds of every `setThresholds` call.
        var thresholds: [Thresholds] = []
        /// Every `restorePoll` call.
        var restoredPolls: [RestoredPoll] = []
        /// Seconds of every `setInterval` call.
        var intervals: [Int64] = []
        /// Number of `startPlanner` and `stopPlanner` calls.
        var plannerStarts = 0
        var plannerStops = 0
        /// Every `importFromCollector` call.
        var imports: [CollectorImportCall] = []
    }

    struct CollectorImportCall: Equatable {
        var collectorNasId: String
        var nasId: String
        var since: Int64?
    }

    struct RestoredPoll: Equatable {
        var nasId: String
        var lastAttemptAt: Int64?
        var blocked: Bool
    }

    private let presets: Mutex<[String: NasState]>
    private let registered = Mutex<[String: NasState]>([:])
    private let log = Mutex(Calls())
    /// Runs inside `addNas`, e.g. a query of the planner finishing then.
    let onAdd = Mutex<(@Sendable () -> Void)?>(nil)
    /// `addNas` throws `InvalidInput` while set.
    let refuseAdd = Mutex(false)
    /// `updateCredentials` throws `UnknownNas` while set.
    let refuseUpdate = Mutex(false)
    /// `refresh` throws `Internal` while set.
    let failRefresh = Mutex(false)
    /// `refresh` waits a little while set, so calls overlap.
    let slowRefresh = Mutex(false)
    /// What `events` returns per NAS id, as the archive would.
    let presetEvents = Mutex<[String: [NasEvent]]>([:])
    /// Saved state → snapshot; `restoreState` refuses any other JSON.
    private let restorable: Mutex<[String: Snapshot]>
    /// Listener of the last `startPlanner`; `stopPlanner` drops it.
    let listener = Mutex<StateListener?>(nil)
    /// What `history` and `forecast` return, for every NAS.
    let historyPreset = Mutex<[Metric: [HistorySeries]]>([:])
    let forecastPreset = Mutex<[String: Forecast]>([:])
    /// What `importFromCollector` returns; it throws `failImport` per
    /// collector NAS id instead.
    let importResult = Mutex(CollectorImport(until: 0, samples: 0, hourly: 0, daily: 0, backupRuns: 0))
    let failImport = Mutex<[String: FfiError]>([:])

    init(states: [NasState] = [], restorable: [String: Snapshot] = [:]) {
        presets = Mutex(Dictionary(uniqueKeysWithValues: states.map { ($0.nasId, $0) }))
        self.restorable = Mutex(restorable)
    }

    /// What `savedState` returns for a NAS with a snapshot.
    static func saved(_ nasId: String) -> String { "saved:\(nasId)" }

    var calls: Calls { log.withLock { $0 } }

    func addNas(config: NasConfig, user: String, password: String) throws {
        log.withLock {
            $0.added.append(config.id)
            $0.addedFingerprints.append(config.fingerprint)
        }
        if let hook = onAdd.withLock({ $0 }) {
            hook()
        }
        if refuseAdd.withLock({ $0 }) {
            throw FfiError.InvalidInput(detail: "kaputt")
        }
        let empty = NasState(nasId: config.id, snapshot: nil, problem: nil, lastAttemptAt: nil, blocked: false)
        registered.withLock { $0[config.id] = empty }
    }

    func refresh(nasId: String) async throws -> NasState {
        log.withLock { $0.refreshed.append(nasId) }
        if slowRefresh.withLock({ $0 }) {
            try? await Task.sleep(for: .milliseconds(50))
        }
        if failRefresh.withLock({ $0 }) {
            throw FfiError.Internal(detail: "weg")
        }
        guard registered.withLock({ $0[nasId] }) != nil else {
            throw FfiError.UnknownNas(id: nasId)
        }
        let state = presets.withLock { $0[nasId] }
            ?? NasState(nasId: nasId, snapshot: nil, problem: nil, lastAttemptAt: 1, blocked: false)
        registered.withLock { $0[nasId] = state }
        return state
    }

    func removeNas(nasId: String) async throws {
        log.withLock { $0.removed.append(nasId) }
        guard registered.withLock({ $0.removeValue(forKey: nasId) }) != nil else {
            throw FfiError.UnknownNas(id: nasId)
        }
    }

    func states() -> [NasState] {
        registered.withLock { $0.values.sorted { $0.nasId < $1.nasId } }
    }

    func savedState(nasId: String) throws -> String? {
        guard let state = registered.withLock({ $0[nasId] }) else {
            throw FfiError.UnknownNas(id: nasId)
        }
        guard let snapshot = state.snapshot else { return nil }
        restorable.withLock { $0[Self.saved(nasId)] = snapshot }
        return Self.saved(nasId)
    }

    func restoreState(nasId: String, json: String) throws {
        log.withLock { $0.restored.append(nasId) }
        guard registered.withLock({ $0[nasId] }) != nil else {
            throw FfiError.UnknownNas(id: nasId)
        }
        guard let snapshot = restorable.withLock({ $0[json] }) else {
            throw FfiError.InvalidInput(detail: "kaputt")
        }
        registered.withLock { states in
            if states[nasId]?.snapshot == nil {
                states[nasId]?.snapshot = snapshot
            }
        }
    }

    func setThresholds(thresholds: Thresholds) {
        log.withLock { $0.thresholds.append(thresholds) }
    }

    func testConnection(config: NasConfig, user: String, password: String) async throws -> Snapshot {
        throw FfiError.Internal(detail: "not used")
    }

    func refreshNow(nasId: String) async throws -> NasState {
        throw FfiError.Internal(detail: "not used")
    }

    func restorePoll(nasId: String, lastAttemptAt: Int64?, blocked: Bool) throws {
        log.withLock {
            $0.restoredPolls.append(RestoredPoll(nasId: nasId, lastAttemptAt: lastAttemptAt, blocked: blocked))
        }
        guard registered.withLock({ $0[nasId] }) != nil else {
            throw FfiError.UnknownNas(id: nasId)
        }
    }

    func startPlanner(listener: StateListener) {
        log.withLock { $0.plannerStarts += 1 }
        self.listener.withLock { $0 = listener }
    }

    func stopPlanner() {
        log.withLock { $0.plannerStops += 1 }
        listener.withLock { $0 = nil }
    }

    /// A finished query of the planner: the state goes into the core, then
    /// to the listener, from a thread of the core.
    func plannerQueried(_ state: NasState) {
        registered.withLock { $0[state.nasId] = state }
        guard let listener = listener.withLock({ $0 }) else { return }
        Thread.detachNewThread { listener.stateChanged(state: state) }
    }

    func setInterval(secs: Int64) {
        log.withLock { $0.intervals.append(secs) }
    }

    func events(nasId: String?, from: Int64, to: Int64) throws -> [NasEvent] {
        presetEvents.withLock { events in
            nasId.map { events[$0] ?? [] } ?? events.values.flatMap { $0 }
        }
    }

    func history(nasId: String, metric: Metric, range: HistoryRange) throws -> [HistorySeries] {
        historyPreset.withLock { $0[metric] ?? [] }
    }

    func forecast(nasId: String, volumeId: String) throws -> Forecast? {
        forecastPreset.withLock { $0[volumeId] }
    }

    func backupRuns(nasId: String, from: Int64, to: Int64) throws -> [BackupRun] { [] }

    func importFromCollector(
        baseUrl: String, fingerprint: String, token: String, collectorNasId: String,
        nasId: String, since: Int64?
    ) async throws -> CollectorImport {
        log.withLock {
            $0.imports.append(CollectorImportCall(collectorNasId: collectorNasId, nasId: nasId, since: since))
        }
        if let error = failImport.withLock({ $0[collectorNasId] }) {
            throw error
        }
        return importResult.withLock { $0 }
    }

    func updateCredentials(nasId: String, user: String, password: String) async throws {
        log.withLock {
            $0.updated.append(nasId)
            $0.updatedUsers.append(user)
            $0.updatedPasswords.append(password)
        }
        if refuseUpdate.withLock({ $0 }) {
            throw FfiError.UnknownNas(id: nasId)
        }
    }
}

/// Keychain double in memory.
final class MemoryAccountStore: AccountStore {
    private let items = Mutex<[UUID: StoredAccount]>([:])
    /// `save` throws while set.
    let refuseSave = Mutex(false)
    /// `loadAll` throws while set.
    let refuseLoad = Mutex(false)

    struct Refused: Error {}

    init(_ stored: [StoredAccount] = []) {
        items.withLock { items in
            for entry in stored {
                items[entry.account.id] = entry
            }
        }
    }

    func loadAll() throws -> [StoredAccount] {
        if refuseLoad.withLock({ $0 }) {
            throw Refused()
        }
        return items.withLock { Array($0.values) }
    }

    func save(_ stored: StoredAccount) throws {
        if refuseSave.withLock({ $0 }) {
            throw Refused()
        }
        items.withLock { $0[stored.account.id] = stored }
    }

    func delete(id: UUID) throws {
        _ = items.withLock { $0.removeValue(forKey: id) }
    }

    func password(of id: UUID) -> String? {
        items.withLock { $0[id]?.password }
    }
}

/// Clock of the store that the tests move on.
final class TestClock {
    var now: Int64 = 1_794_665_100

    func advance(minutes: Int64) {
        now += minutes * 60
    }
}

@MainActor
struct NasStoreTests {
    let defaults: UserDefaults
    let clock = TestClock()

    init() throws {
        defaults = try #require(UserDefaults(suiteName: "nasstats-tests-\(UUID().uuidString)"))
        // Without a stored setting, the store moves every NAS to iCloud
        // Keychain at its first start (#113); the tests of that remove it.
        defaults.set(false, forKey: NasStore.sharesAccessViaICloudKey)
    }

    static func account(_ name: String, provider: String = NasAccount.qnapKey) -> StoredAccount {
        StoredAccount(
            account: NasAccount(
                id: UUID(),
                name: name,
                providerKey: provider,
                baseUrl: "https://nas.demo.invalid",
                fingerprint: String(repeating: "AB", count: 32),
                user: "monitor",
                model: nil,
                syncsViaICloud: false
            ),
            password: "demo"
        )
    }

    static func snapshot(_ id: String, health: Health?) -> Snapshot {
        var snapshot = demoSnapshot()
        snapshot.nas.id = id
        snapshot.demo = false
        snapshot.worstHealth = health
        // The demo snapshot has findings, which would outrank `health`.
        snapshot.findings = []
        return snapshot
    }

    static func state(
        _ account: StoredAccount,
        health: Health?,
        problem: NasProblem? = nil,
        blocked: Bool = false
    ) -> NasState {
        let id = account.account.id.uuidString
        return NasState(
            nasId: id,
            snapshot: health.map { snapshot(id, health: $0) },
            problem: problem,
            lastAttemptAt: 100,
            blocked: blocked
        )
    }

    static func problem(_ kind: ProblemKind, stops: Bool) -> NasProblem {
        NasProblem(
            kind: kind, message: "Fehler", stopsPolling: stops,
            noAnswer: kind == .unreachable || kind == .timeout,
            oldFingerprint: nil, newFingerprint: nil, httpStatus: nil
        )
    }

    /// Starts a store on `core` with `keychain`.
    func started(_ keychain: MemoryAccountStore, core: FakeCore) async -> NasStore {
        let store = NasStore(
            accounts: NasAccounts(keychain: keychain, core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now }
        )
        store.start()
        // The planner of the fake core never queries; this is the first
        // round the planner of the real core would run.
        await store.refreshAll()
        return store
    }

    /// A file for `LastStates` that the test removes again.
    static func lastStatesFile() -> LastStates {
        LastStates(fileURL: FileManager.default.temporaryDirectory
            .appendingPathComponent("last-states-\(UUID().uuidString).json"))
    }

    /// A store on `core` with `keychain` and `lastStates`, not started.
    func store(_ keychain: MemoryAccountStore, core: FakeCore, lastStates: LastStates) -> NasStore {
        NasStore(
            accounts: NasAccounts(keychain: keychain, core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            lastStates: lastStates
        )
    }

    @Test func aQueryNotifiesTheOpenEventsAndRemoveWithdrawsThem() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(a, health: .warning)])
        let open = NasEvent(
            nasId: id, kind: .diskTemperature, subject: "3", level: .warning,
            text: "Platte 3 bei 52 °C", startedAt: 1_000, endedAt: nil
        )
        let ended = NasEvent(
            nasId: id, kind: .upsOnBattery, subject: nil, level: .warning,
            text: "USV im Akkubetrieb", startedAt: 900, endedAt: 950
        )
        core.presetEvents.withLock { $0[id] = [open, ended] }
        let poster = FakePoster()
        let store = NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore([a]), core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            notifier: EventNotifier(poster: poster, defaults: defaults)
        )
        store.start()
        await store.refreshAll()
        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: open)])
        #expect(poster.posts.first?.title == "Keller")

        // The same state on the next query notifies nothing new.
        clock.advance(minutes: 5)
        await store.refreshAll()
        #expect(core.calls.refreshed == [id, id])
        #expect(poster.posts.count == 1)

        try await store.remove(id: a.account.id)
        #expect(poster.withdrawn == [EventNotifier.id(of: open)])
    }

    @Test func switchingAKindOffWithdrawsItsNotification() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(a, health: .warning)])
        let open = NasEvent(
            nasId: id, kind: .diskTemperature, subject: "3", level: .warning,
            text: "Platte 3 bei 52 °C", startedAt: 1_000, endedAt: nil
        )
        core.presetEvents.withLock { $0[id] = [open] }
        let poster = FakePoster()
        let store = NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore([a]), core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            notifier: EventNotifier(poster: poster, defaults: defaults)
        )
        store.start()
        await store.refreshAll()
        store.notificationSettings.disabled = [.disks]
        #expect(poster.withdrawn == [EventNotifier.id(of: open)])
    }

    @Test func onceAllowedTheOpenEventsNotifyWithoutAQuery() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore(states: [Self.state(a, health: .warning)])
        let open = NasEvent(
            nasId: id, kind: .diskTemperature, subject: "3", level: .warning,
            text: "Platte 3 bei 52 °C", startedAt: 1_000, endedAt: nil
        )
        core.presetEvents.withLock { $0[id] = [open] }
        // The user decides only after the first query.
        let poster = FakePoster()
        poster.authorized = false
        let store = NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore([a]), core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            notifier: EventNotifier(poster: poster, defaults: defaults)
        )
        store.start()
        await store.refreshAll()
        await store.permissionRequest?.value
        #expect(poster.posts.isEmpty)

        poster.grantsOnRequest = true
        try await store.add(Self.account("Büro"))
        await store.permissionRequest?.value
        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: open)])
        #expect(core.calls.refreshed == [id])
    }

    @Test func aStateOfThePlannerNotifiesItsOpenEvents() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore()
        let open = NasEvent(
            nasId: id, kind: .diskTemperature, subject: "3", level: .warning,
            text: "Platte 3 bei 52 °C", startedAt: 1_000, endedAt: nil
        )
        core.presetEvents.withLock { $0[id] = [open] }
        let poster = FakePoster()
        let store = NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore([a]), core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            notifier: EventNotifier(poster: poster, defaults: defaults)
        )
        store.start()
        await store.permissionRequest?.value
        core.plannerQueried(Self.state(a, health: .warning))
        #expect(await eventually { !poster.posts.isEmpty })
        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: open)])
        #expect(core.calls.refreshed.isEmpty)
    }

    @Test func theNotificationSettingsAreSaved() async throws {
        let store = await started(MemoryAccountStore(), core: FakeCore())
        store.notificationSettings.disabled = [.ups]
        #expect(NotificationSettings.load(from: defaults).disabled == [.ups])
    }

    @Test func theLastStateComesBackAfterARestart() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let file = Self.lastStatesFile()
        defer { try? FileManager.default.removeItem(at: file.fileURL!) }
        file.save([id: FakeCore.saved(id)])

        // Away from home: the query after the start fails.
        let core = FakeCore(restorable: [FakeCore.saved(id): Self.snapshot(id, health: .ok)])
        core.failRefresh.withLock { $0 = true }
        let store = store(MemoryAccountStore([a]), core: core, lastStates: file)
        #expect(core.calls.restored == [id])
        #expect(store.entries.first?.snapshot != nil)
        store.start()
        await store.refreshAll()
        #expect(core.calls.refreshed == [id])
        #expect(store.entries.first?.snapshot?.nas.id == id)
        #expect(file.load() == [id: FakeCore.saved(id)])
    }

    @Test func aStateTheCoreRefusesIsDropped() async throws {
        let (a, b) = (Self.account("A"), Self.account("B"))
        let (idA, idB) = (a.account.id.uuidString, b.account.id.uuidString)
        let file = Self.lastStatesFile()
        defer { try? FileManager.default.removeItem(at: file.fileURL!) }
        file.save([idA: "altes Format", idB: FakeCore.saved(idB)])

        let core = FakeCore(restorable: [FakeCore.saved(idB): Self.snapshot(idB, health: .ok)])
        let store = store(MemoryAccountStore([a, b]), core: core, lastStates: file)
        #expect(file.load() == [idB: FakeCore.saved(idB)])
        #expect(store.entries.first { $0.id == idA }?.snapshot == nil)
        #expect(store.entries.first { $0.id == idB }?.snapshot != nil)
    }

    @Test func aQuerySavesTheStateAndRemoveDeletesIt() async throws {
        let (a, b) = (Self.account("A"), Self.account("B"))
        let idA = a.account.id.uuidString
        let file = Self.lastStatesFile()
        defer { try? FileManager.default.removeItem(at: file.fileURL!) }

        // B answers without a snapshot, so nothing is saved for it.
        let core = FakeCore(states: [Self.state(a, health: .ok), Self.state(b, health: nil)])
        let store = store(MemoryAccountStore([a, b]), core: core, lastStates: file)
        store.start()
        await store.refreshAll()
        #expect(file.load() == [idA: FakeCore.saved(idA)])

        try await store.remove(id: a.account.id)
        #expect(file.load() == [:])
    }

    @Test func aSecondRefreshWaitsForTheRunningRound() async throws {
        let a = Self.account("Keller")
        let core = FakeCore(states: [Self.state(a, health: .ok)])
        core.slowRefresh.withLock { $0 = true }
        let store = store(MemoryAccountStore([a]), core: core, lastStates: LastStates(fileURL: nil))

        // Runs once the first call waits for its round.
        let second = Task { @MainActor in
            await store.refreshAll()
            return store.entries.first?.snapshot != nil
        }
        await store.refreshAll()
        #expect(await second.value)
        #expect(core.calls.refreshed == [a.account.id.uuidString])
    }

    @Test func aCancelledRefreshReturnsWhileTheRoundRunsOn() async throws {
        let a = Self.account("Keller")
        let core = FakeCore(states: [Self.state(a, health: .ok)])
        core.slowRefresh.withLock { $0 = true }
        let store = store(MemoryAccountStore([a]), core: core, lastStates: LastStates(fileURL: nil))

        // Like the background task that iOS ends early.
        let cancelled = Task { @MainActor in
            await store.refreshAll()
            return store.entries.first?.snapshot != nil
        }
        cancelled.cancel()
        #expect(await cancelled.value == false)

        await store.refreshAll()
        #expect(store.entries.first?.snapshot != nil)
        #expect(core.calls.refreshed == [a.account.id.uuidString])
    }

    @Test func theStateOfANasNoLongerStoredIsDropped() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let gone = UUID().uuidString
        let file = Self.lastStatesFile()
        defer { try? FileManager.default.removeItem(at: file.fileURL!) }
        file.save([id: FakeCore.saved(id), gone: FakeCore.saved(gone)])

        let core = FakeCore(restorable: [FakeCore.saved(id): Self.snapshot(id, health: .ok)])
        _ = store(MemoryAccountStore([a]), core: core, lastStates: file)
        #expect(file.load() == [id: FakeCore.saved(id)])
    }

    @Test func sortsCriticalWarningOkUnknown() async throws {
        let (a, b, c, d) = (Self.account("Anton"), Self.account("Berta"), Self.account("Cäsar"), Self.account("Dora"))
        let core = FakeCore(states: [
            Self.state(a, health: .ok),
            Self.state(b, health: nil),
            Self.state(c, health: .warning),
            Self.state(d, health: .critical),
        ])
        let store = await started(MemoryAccountStore([a, b, c, d]), core: core)
        #expect(store.entries.map(\.displayName) == ["Dora", "Cäsar", "Anton", "Berta"])
        #expect(store.entries.map(\.health) == [.critical, .warning, .ok, .unknown])
    }

    @Test func aFindingLiftsOkToWarning() async throws {
        let (a, b) = (Self.account("Anton"), Self.account("Berta"))
        var hot = Self.state(b, health: .ok)
        hot.snapshot?.findings = [
            Finding(kind: .alert, level: .warning, subject: nil, text: "Lüfter laut"),
        ]
        let core = FakeCore(states: [Self.state(a, health: .ok), hot])
        let store = await started(MemoryAccountStore([a, b]), core: core)
        #expect(store.entries.map(\.displayName) == ["Berta", "Anton"])
        #expect(store.entries.map(\.health) == [.warning, .ok])
    }

    @Test func aCriticalNasStaysCriticalWithAWarningFinding() async throws {
        let a = Self.account("Anton")
        var state = Self.state(a, health: .critical)
        state.snapshot?.findings = [
            Finding(kind: .alert, level: .warning, subject: nil, text: "Update verfügbar"),
        ]
        let store = await started(MemoryAccountStore([a]), core: FakeCore(states: [state]))
        #expect(store.entries.first?.health == .critical)
    }

    @Test func sameHealthSortsByName() async throws {
        let (x, y) = (Self.account("Zeta"), Self.account("alpha"))
        let core = FakeCore(states: [Self.state(x, health: .ok), Self.state(y, health: .ok)])
        let store = await started(MemoryAccountStore([x, y]), core: core)
        #expect(store.entries.map(\.displayName) == ["alpha", "Zeta"])
    }

    @Test func errorKeepsTheSnapshotAndLiftsToWarning() async throws {
        let a = Self.account("Keller")
        let core = FakeCore(states: [
            Self.state(a, health: .ok, problem: Self.problem(.unreachable, stops: false)),
        ])
        let store = await started(MemoryAccountStore([a]), core: core)
        let entry = try #require(store.entries.first)
        #expect(entry.snapshot != nil)
        #expect(entry.observedAt != nil)
        #expect(entry.problem?.kind == .unreachable)
        #expect(entry.health == .warning)
    }

    @Test func blockedNasIsCritical() async throws {
        let a = Self.account("Keller")
        let core = FakeCore(states: [
            Self.state(a, health: .ok, problem: Self.problem(.loginFailed, stops: true), blocked: true),
        ])
        let store = await started(MemoryAccountStore([a]), core: core)
        #expect(store.entries.first?.health == .critical)
    }

    @Test func refusedRegistrationIsCriticalAndNotQueried() async throws {
        let core = FakeCore()
        core.refuseAdd.withLock { $0 = true }
        let store = await started(MemoryAccountStore([Self.account("Keller")]), core: core)
        let entry = try #require(store.entries.first)
        #expect(entry.localError == "Ungültige Angabe: kaputt")
        #expect(entry.health == .critical)
        #expect(core.calls.refreshed.isEmpty)
    }

    @Test func unknownProviderIsCriticalAndNotRegistered() async throws {
        let core = FakeCore()
        let store = await started(MemoryAccountStore([Self.account("Alt", provider: "omv")]), core: core)
        let entry = try #require(store.entries.first)
        #expect(entry.localError == "Ungültige Angabe: Anbieter omv unbekannt")
        #expect(entry.health == .critical)
        #expect(core.calls.added.isEmpty)
    }

    @Test func startQueriesTheStoredNas() async throws {
        let (a, b) = (Self.account("A"), Self.account("B"))
        let core = FakeCore()
        let store = await started(MemoryAccountStore([a, b]), core: core)
        let ids = [a, b].map(\.account.id.uuidString).sorted()
        #expect(core.calls.added.sorted() == ids)
        #expect(core.calls.refreshed.sorted() == ids)
        #expect(store.entries.count == 2)

        #expect(core.calls.plannerStarts == 1)

        // A second start does nothing.
        let calls = core.calls
        store.start()
        #expect(core.calls.restoredPolls == calls.restoredPolls)
        #expect(core.calls.plannerStarts == 1)
        #expect(core.calls.intervals == calls.intervals)
    }

    @Test func addAndRemoveChangeKeychainCoreAndEntries() async throws {
        let core = FakeCore()
        let keychain = MemoryAccountStore()
        let store = await started(keychain, core: core)
        let new = Self.account("Neu")
        let id = new.account.id

        // The test query of the dialog counts as the first query.
        try await store.add(new, snapshot: Self.snapshot(id.uuidString, health: .warning))
        #expect(keychain.password(of: id) == "demo")
        #expect(core.calls.added == [id.uuidString])
        #expect(core.calls.refreshed.isEmpty)
        #expect(store.entries.map(\.displayName) == ["Neu"])
        #expect(store.entries.first?.health == .warning)

        try await store.remove(id: id)
        #expect(keychain.password(of: id) == nil)
        #expect(core.calls.removed == [id.uuidString])
        #expect(store.entries.isEmpty)
    }

    @Test func addChangesNothingWhenTheKeychainRefuses() async throws {
        let core = FakeCore()
        let keychain = MemoryAccountStore()
        keychain.refuseSave.withLock { $0 = true }
        let store = await started(keychain, core: core)
        await #expect(throws: MemoryAccountStore.Refused.self) {
            try await store.add(Self.account("Neu"))
        }
        #expect(core.calls.added.isEmpty)
        #expect(store.entries.isEmpty)
    }

    @Test func addRemovesTheKeychainEntryWhenTheCoreRefuses() async throws {
        let core = FakeCore()
        let keychain = MemoryAccountStore()
        let store = await started(keychain, core: core)
        core.refuseAdd.withLock { $0 = true }
        let new = Self.account("Neu")
        await #expect(throws: FfiError.InvalidInput(detail: "kaputt")) {
            try await store.add(new)
        }
        #expect(keychain.password(of: new.account.id) == nil)
        #expect(store.entries.isEmpty)
    }

    @Test func newPasswordLiftsTheBlockAndQueries() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        let keychain = MemoryAccountStore([a])
        let store = await started(keychain, core: core)
        try await store.updateCredentials(id: a.account.id, user: "monitor", password: "neu")
        #expect(core.calls.updated == [a.account.id.uuidString])
        #expect(core.calls.refreshed.count == 2)
        #expect(keychain.password(of: a.account.id) == "neu")
    }

    @Test func newUserIsStoredAndUsed() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let core = FakeCore()
        let store = await started(keychain, core: core)
        try await store.updateCredentials(id: a.account.id, user: "leser", password: "neu")
        #expect(core.calls.updatedUsers == ["leser"])
        let stored = try #require(try keychain.loadAll().first)
        #expect(stored.account.user == "leser")
        #expect(stored.password == "neu")
        #expect(store.accounts.accounts.first?.user == "leser")
    }

    @Test func refusedNewUserKeepsTheOldAccount() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        core.refuseUpdate.withLock { $0 = true }
        let keychain = MemoryAccountStore([a])
        let store = await started(keychain, core: core)
        await #expect(throws: FfiError.UnknownNas(id: a.account.id.uuidString)) {
            try await store.updateCredentials(id: a.account.id, user: "leser", password: "neu")
        }
        #expect(try keychain.loadAll().first?.account.user == "monitor")
        #expect(store.accounts.accounts.first?.user == "monitor")
    }

    @Test func newPasswordRegistersARefusedNas() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        core.refuseAdd.withLock { $0 = true }
        let keychain = MemoryAccountStore([a])
        let store = await started(keychain, core: core)
        core.refuseAdd.withLock { $0 = false }

        try await store.updateCredentials(id: a.account.id, user: "monitor", password: "neu")
        #expect(core.calls.added.count == 2)
        #expect(core.calls.refreshed == [a.account.id.uuidString])
        #expect(store.entries.first?.localError == nil)
        #expect(keychain.password(of: a.account.id) == "neu")
    }

    @Test func refusedNewPasswordKeepsTheOldOne() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        core.refuseUpdate.withLock { $0 = true }
        let keychain = MemoryAccountStore([a])
        let store = await started(keychain, core: core)
        await #expect(throws: FfiError.UnknownNas(id: a.account.id.uuidString)) {
            try await store.updateCredentials(id: a.account.id, user: "monitor", password: "neu")
        }
        #expect(keychain.password(of: a.account.id) == "demo")
    }

    @Test func refusedRegistrationOnNewPasswordShowsTheReason() async throws {
        let a = Self.account("Keller")
        let core = FakeCore()
        core.refuseAdd.withLock { $0 = true }
        let keychain = MemoryAccountStore([a])
        let store = await started(keychain, core: core)
        await #expect(throws: FfiError.InvalidInput(detail: "kaputt")) {
            try await store.updateCredentials(id: a.account.id, user: "monitor", password: "neu")
        }
        #expect(store.entries.first?.localError == "Ungültige Angabe: kaputt")
        #expect(keychain.password(of: a.account.id) == "demo")
    }

    @Test func aBlockHoldsAcrossLaunches() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let first = FakeCore(states: [
            Self.state(a, health: nil, problem: Self.problem(.loginFailed, stops: true), blocked: true),
        ])
        _ = await started(keychain, core: first)
        #expect(first.calls.refreshed.count == 1)

        // Next launch, an hour later: a fresh core knows no block, the
        // store does, with its reason.
        clock.advance(minutes: 60)
        let second = FakeCore()
        let store = await started(keychain, core: second)
        #expect(second.calls.refreshed.isEmpty)
        await store.refreshAll()
        #expect(second.calls.refreshed.isEmpty)
        #expect(store.entries.first?.health == .critical)
        #expect(store.entries.first?.localError == "Fehler")

        // A new password lifts it.
        try await store.updateCredentials(id: a.account.id, user: "monitor", password: "neu")
        #expect(second.calls.refreshed == [a.account.id.uuidString])
    }

    @Test func aLaunchWithinFiveMinutesDoesNotQueryAgain() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        _ = await started(keychain, core: FakeCore(states: [Self.state(a, health: .ok)]))

        clock.advance(minutes: 4)
        let second = FakeCore()
        let store = await started(keychain, core: second)
        #expect(second.calls.refreshed.isEmpty)
        #expect(store.entries.first?.lastAttemptAt == clock.now - 4 * 60)

        // Not before 5 minutes, also not after leaving demo mode.
        store.demoMode = true
        store.demoMode = false
        await store.refreshAll()
        #expect(second.calls.refreshed.isEmpty)
        clock.advance(minutes: 1)
        await store.refreshAll()
        #expect(second.calls.refreshed == [a.account.id.uuidString])
    }

    @Test func theTestQueryOfTheDialogCountsAsAQuery() async throws {
        let core = FakeCore()
        let store = await started(MemoryAccountStore(), core: core)
        let new = Self.account("Neu")
        try await store.add(new, snapshot: Self.snapshot(new.account.id.uuidString, health: .ok))
        clock.advance(minutes: 4)
        await store.refreshAll()
        #expect(core.calls.refreshed.isEmpty)
        clock.advance(minutes: 1)
        await store.refreshAll()
        #expect(core.calls.refreshed == [new.account.id.uuidString])
    }

    @Test func storedNasShowBeforeTheFirstQuery() {
        let store = NasStore(
            accounts: NasAccounts(keychain: MemoryAccountStore([Self.account("Keller")]), core: FakeCore()),
            defaults: defaults,
            demoSource: { _, _ in [] }
        )
        #expect(store.entries.map(\.displayName) == ["Keller"])
    }

    @Test func thrownQueryIsAWarningNotCritical() async throws {
        let core = FakeCore()
        core.failRefresh.withLock { $0 = true }
        let store = await started(MemoryAccountStore([Self.account("Keller")]), core: core)
        let entry = try #require(store.entries.first)
        #expect(entry.queryError != nil)
        #expect(entry.localError == nil)
        #expect(entry.health == .warning)

        core.failRefresh.withLock { $0 = false }
        clock.advance(minutes: 5)
        await store.refreshAll()
        #expect(store.entries.first?.queryError == nil)
    }

    @Test func demoModeShowsTheThreeDemoNasWithoutQueries() async throws {
        defaults.set(true, forKey: NasStore.demoModeKey)
        let a = Self.account("Keller echt")
        let core = FakeCore()
        let store = await started(MemoryAccountStore([a]), core: core)
        // Registered, but not queried.
        #expect(core.calls.added == [a.account.id.uuidString])
        await store.refreshAll()
        #expect(core.calls.refreshed.isEmpty)

        #expect(store.entries.map(\.id).sorted() == ["demo-buero", "demo-homelab", "demo-keller"])
        #expect(store.entries.allSatisfy { $0.snapshot?.demo == true })
        // Keller has a hot disk and a full volume and comes first.
        #expect(store.entries.first?.id == "demo-keller")
        #expect(store.entries.map(\.health) == [.warning, .ok, .ok])
        #expect(store.entries.map(\.provider) == [.qnap, .synology, .trueNas])

        store.demoMode = false
        #expect(!defaults.bool(forKey: NasStore.demoModeKey))
        #expect(store.entries.map(\.displayName) == ["Keller echt"])
    }

    @Test func onUpdateFollowsEveryNewState() async throws {
        let a = Self.account("Keller")
        let core = FakeCore(states: [Self.state(a, health: .ok)])
        let store = await started(MemoryAccountStore([a]), core: core)
        var reported: [[String]] = []
        store.onUpdate = { reported.append($0.map(\.id)) }

        clock.advance(minutes: 5)
        await store.refreshAll()
        #expect(reported == [[a.account.id.uuidString]])

        store.demoMode = true
        #expect(reported.count == 2)
        #expect(reported.last?.count == 3)
    }

    // MARK: - Settings (#21)

    func makeStore(_ keychain: MemoryAccountStore, core: FakeCore) -> NasStore {
        NasStore(
            accounts: NasAccounts(keychain: keychain, core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now }
        )
    }

    @Test func theIntervalSpacesTheQueriesAndOutlivesARestart() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let core = FakeCore()
        let first = await started(keychain, core: core)
        #expect(core.calls.refreshed.count == 1)
        first.queryIntervalMinutes = 15
        // A change alone queries nothing.
        #expect(core.calls.refreshed.count == 1)

        let second = makeStore(keychain, core: core)
        #expect(second.queryIntervalMinutes == 15)
        clock.advance(minutes: 10)
        await second.refreshAll()
        #expect(core.calls.refreshed.count == 1)
        clock.advance(minutes: 5)
        await second.refreshAll()
        #expect(core.calls.refreshed.count == 2)
    }

    @Test func anUnknownIntervalIsFiveMinutes() {
        defaults.set(7, forKey: NasStore.queryIntervalKey)
        let store = makeStore(MemoryAccountStore(), core: FakeCore())
        #expect(store.queryIntervalMinutes == 5)
    }

    @Test func theCoreGetsTheThresholdsAtStartAndAfterAChange() {
        AlertThresholds(fill: FillThresholds(warning: 70, critical: 85), diskTempWarningC: 45)
            .save(to: defaults)
        let core = FakeCore()
        let store = makeStore(MemoryAccountStore(), core: core)
        #expect(core.calls.thresholds == [
            Thresholds(diskTempWarningC: 45, fillWarningPercent: 70, fillCriticalPercent: 85),
        ])
        store.thresholds.diskTempWarningC = 55
        #expect(core.calls.thresholds.last
            == Thresholds(diskTempWarningC: 55, fillWarningPercent: 70, fillCriticalPercent: 85))
        #expect(core.calls.thresholds.count == 2)
        // The same thresholds again change nothing.
        store.thresholds.diskTempWarningC = 55
        #expect(core.calls.thresholds.count == 2)
    }

    @Test func changedThresholdsReachTheFindingsOfTheCore() throws {
        let store = makeStore(MemoryAccountStore(), core: FakeCore())
        store.demoMode = true
        func keller() throws -> NasEntry {
            try #require(store.entries.first { $0.id == "demo-keller" })
        }
        #expect(try keller().findings.map(\.text) == ["Platte 3 bei 52 °C", "DataVol1 bei 87 %"])
        #expect(try keller().health == .warning)

        // Hotter disks allowed, critical from 85 %.
        store.thresholds = AlertThresholds(
            fill: FillThresholds(warning: 75, critical: 85), diskTempWarningC: 55
        )
        #expect(try keller().findings.map(\.text) == ["DataVol1 bei 87 %"])
        #expect(try keller().findings.first?.level == .critical)
        #expect(try keller().health == .critical)
        #expect(store.entries.first?.id == "demo-keller")
        #expect(store.entries.allSatisfy { $0.thresholds == store.thresholds })

        store.thresholds = AlertThresholds(
            fill: FillThresholds(warning: 90, critical: 95), diskTempWarningC: 60
        )
        #expect(try keller().findings.isEmpty)
    }

    @Test func theThresholdsOutliveARestart() {
        let first = makeStore(MemoryAccountStore(), core: FakeCore())
        #expect(first.thresholds == .standard)
        first.thresholds = AlertThresholds(
            fill: FillThresholds(warning: 70, critical: 85), diskTempWarningC: 45
        )
        let second = makeStore(MemoryAccountStore(), core: FakeCore())
        #expect(second.thresholds.fill == FillThresholds(warning: 70, critical: 85))
        #expect(second.thresholds.diskTempWarningC == 45)
    }

    @Test func invalidStoredThresholdsAreTheStandard() {
        AlertThresholds(fill: FillThresholds(warning: 90, critical: 80), diskTempWarningC: 50)
            .save(to: defaults)
        #expect(makeStore(MemoryAccountStore(), core: FakeCore()).thresholds == .standard)
        // Closer than one step: the steppers would get an empty range.
        AlertThresholds(fill: FillThresholds(warning: 50, critical: 52), diskTempWarningC: 50)
            .save(to: defaults)
        #expect(makeStore(MemoryAccountStore(), core: FakeCore()).thresholds == .standard)
        defaults.set(Data("kaputt".utf8), forKey: AlertThresholds.defaultsKey)
        #expect(makeStore(MemoryAccountStore(), core: FakeCore()).thresholds == .standard)
    }

    @Test func renameChangesOnlyTheName() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let core = FakeCore()
        let store = await started(keychain, core: core)
        let calls = core.calls
        try store.rename(id: a.account.id, to: "  Büro  ")
        let stored = try #require(try keychain.loadAll().first)
        #expect(stored.account.name == "Büro")
        #expect(stored.password == "demo")
        #expect(store.entries.first?.displayName == "Büro")
        #expect(core.calls.added == calls.added)
        #expect(core.calls.updated == calls.updated)
        #expect(core.calls.refreshed == calls.refreshed)

        #expect(throws: FfiError.InvalidInput(detail: "Der Name darf nicht leer sein.")) {
            try store.rename(id: a.account.id, to: " ")
        }
        #expect(throws: FfiError.self) {
            try store.rename(id: UUID(), to: "Fremd")
        }
    }

    @Test func theICloudSettingMovesEveryNasBothWays() async throws {
        var a = Self.account("Keller")
        a.account.syncsViaICloud = true
        let b = Self.account("Büro")
        let keychain = MemoryAccountStore([a, b])
        let core = FakeCore()
        let store = await started(keychain, core: core)
        let calls = core.calls

        for shares in [false, true] {
            #expect(try store.setSharesAccessViaICloud(shares) == 0)
            #expect(store.sharesAccessViaICloud == shares)
            #expect(defaults.object(forKey: NasStore.sharesAccessViaICloudKey) as? Bool == shares)
            let stored = try keychain.loadAll()
            #expect(stored.count == 2)
            #expect(stored.allSatisfy { $0.account.syncsViaICloud == shares })
            #expect(stored.allSatisfy { $0.password == "demo" })
            #expect(store.accounts.accounts.allSatisfy { $0.syncsViaICloud == shares })
            #expect(core.calls.added == calls.added)
            #expect(core.calls.updated == calls.updated)
            #expect(core.calls.refreshed == calls.refreshed)
        }
    }

    @Test func aRefusedICloudSwitchKeepsTheEntriesAndSavesTheSetting() async throws {
        let a = Self.account("Keller")
        let b = Self.account("Büro")
        let keychain = MemoryAccountStore([a, b])
        let store = await started(keychain, core: FakeCore())
        keychain.refuseSave.withLock { $0 = true }

        #expect(try store.setSharesAccessViaICloud(true) == 2)
        #expect(try keychain.loadAll().allSatisfy { !$0.account.syncsViaICloud })
        #expect(store.accounts.accounts.allSatisfy { !$0.syncsViaICloud })
        #expect(store.sharesAccessViaICloud)
        #expect(defaults.object(forKey: NasStore.sharesAccessViaICloudKey) as? Bool == true)
    }

    @Test func theFirstStartSharesTheNasKeptOnThisDevice() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        defaults.removeObject(forKey: NasStore.sharesAccessViaICloudKey)
        let store = await started(keychain, core: FakeCore())

        #expect(store.sharesAccessViaICloud)
        #expect(defaults.object(forKey: NasStore.sharesAccessViaICloudKey) as? Bool == true)
        #expect(try keychain.loadAll().first?.account.syncsViaICloud == true)
        #expect(try keychain.loadAll().first?.password == "demo")
        #expect(store.accounts.accounts.first?.syncsViaICloud == true)
    }

    @Test func anUnreadableKeychainLeavesTheICloudSettingForTheNextStart() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        keychain.refuseLoad.withLock { $0 = true }
        defaults.removeObject(forKey: NasStore.sharesAccessViaICloudKey)
        let store = await started(keychain, core: FakeCore())

        #expect(store.loadError != nil)
        #expect(defaults.object(forKey: NasStore.sharesAccessViaICloudKey) == nil)
        #expect(throws: FfiError.InvalidInput(detail: "Die gespeicherten NAS ließen sich nicht aus dem Schlüsselbund lesen.")) {
            try store.setSharesAccessViaICloud(false)
        }
        #expect(store.sharesAccessViaICloud)
        #expect(defaults.object(forKey: NasStore.sharesAccessViaICloudKey) == nil)

        // The next start with a readable keychain moves the NAS.
        keychain.refuseLoad.withLock { $0 = false }
        let next = await started(keychain, core: FakeCore())
        #expect(next.loadError == nil)
        #expect(defaults.object(forKey: NasStore.sharesAccessViaICloudKey) as? Bool == true)
        #expect(try keychain.loadAll().first?.account.syncsViaICloud == true)
    }

    @Test func aStoredOffKeepsTheNasOnThisDevice() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let store = await started(keychain, core: FakeCore())

        #expect(!store.sharesAccessViaICloud)
        #expect(try keychain.loadAll().first?.account.syncsViaICloud == false)
        #expect(store.accounts.accounts.first?.syncsViaICloud == false)
    }

    @Test(arguments: [true, false])
    func addFollowsTheICloudSetting(shares: Bool) async throws {
        defaults.set(shares, forKey: NasStore.sharesAccessViaICloudKey)
        let keychain = MemoryAccountStore()
        let store = await started(keychain, core: FakeCore())
        var new = Self.account("Keller")
        new.account.syncsViaICloud = !shares

        try await store.add(new)

        #expect(try keychain.loadAll().first?.account.syncsViaICloud == shares)
        #expect(store.accounts.accounts.first?.syncsViaICloud == shares)
    }

    @Test func resumeStartsABlockedNasWithTheStoredAccess() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let core = FakeCore(states: [
            Self.state(a, health: nil, problem: Self.problem(.loginFailed, stops: true), blocked: true),
        ])
        let store = await started(keychain, core: core)
        #expect(store.entries.first?.blocked == true)
        #expect(core.calls.refreshed.count == 1)

        // The NAS takes the login now.
        let unblocked = FakeCore()
        let fresh = await started(keychain, core: unblocked)
        try await fresh.resume(id: a.account.id)
        #expect(unblocked.calls.updatedUsers == ["monitor"])
        #expect(unblocked.calls.updatedPasswords == ["demo"])
        #expect(unblocked.calls.refreshed == [a.account.id.uuidString])
        #expect(fresh.entries.first?.blocked == false)
        #expect(keychain.password(of: a.account.id) == "demo")
    }

    @Test func confirmingANewCertificateRegistersAgainAndQueries() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        // Blocked by a changed certificate in an earlier launch.
        _ = await started(keychain, core: FakeCore(states: [
            Self.state(a, health: nil, problem: Self.problem(.certificateChanged, stops: true), blocked: true),
        ]))
        clock.advance(minutes: 60)
        let core = FakeCore()
        let store = await started(keychain, core: core)
        #expect(store.entries.first?.blocked == true)
        #expect(core.calls.refreshed.isEmpty)
        let newPrint = String(repeating: "CD", count: 32)

        try await store.confirmFingerprint(id: a.account.id, fingerprint: newPrint)
        #expect(try keychain.loadAll().first?.account.fingerprint == newPrint)
        #expect(core.calls.addedFingerprints.last == newPrint)
        #expect(core.calls.refreshed == [a.account.id.uuidString])
        #expect(store.entries.first?.blocked == false)
        #expect(store.accounts.accounts.first?.fingerprint == newPrint)
    }

    @Test func aRefusedNewCertificateKeepsTheOldOne() async throws {
        let a = Self.account("Keller")
        let keychain = MemoryAccountStore([a])
        let core = FakeCore()
        let store = await started(keychain, core: core)
        core.refuseAdd.withLock { $0 = true }
        await #expect(throws: FfiError.InvalidInput(detail: "kaputt")) {
            try await store.confirmFingerprint(id: a.account.id, fingerprint: String(repeating: "CD", count: 32))
        }
        #expect(try keychain.loadAll().first?.account.fingerprint == a.account.fingerprint)
        #expect(store.accounts.accounts.first?.fingerprint == a.account.fingerprint)
        #expect(store.entries.first?.localError == "Ungültige Angabe: kaputt")
        #expect(core.calls.refreshed.count == 1)
    }

    @Test func aFailedResumeIsNotRetriedByItself() async throws {
        let a = Self.account("Keller")
        let blocked = Self.state(a, health: nil, problem: Self.problem(.loginFailed, stops: true), blocked: true)
        let core = FakeCore(states: [blocked])
        let store = await started(MemoryAccountStore([a]), core: core)
        #expect(core.calls.refreshed.count == 1)

        try await store.resume(id: a.account.id)
        #expect(core.calls.refreshed.count == 2)
        #expect(store.entries.first?.blocked == true)

        // Blocked again: no further query until the user acts.
        clock.advance(minutes: 60)
        await store.refreshAll()
        #expect(core.calls.refreshed.count == 2)
    }

    // MARK: - History, forecast and the planner (#67)

    /// Waits up to 2 s for `condition`, e.g. for a state of the planner.
    func eventually(_ condition: () -> Bool) async -> Bool {
        for _ in 0..<200 {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return condition()
    }

    @Test func theDemoForecastOfDataVol1IsFullIn143Days() throws {
        defaults.set(true, forKey: NasStore.demoModeKey)
        let store = makeStore(MemoryAccountStore(), core: FakeCore())
        let keller = try #require(store.entries.first { $0.id == "demo-keller" })
        let forecast = try #require(keller.forecast(for: "demo-keller-datavol1"))
        let text = StatusFormat.forecast(forecast)
        #expect(text.main == "voll in 143 Tagen")
        #expect(text.date != nil)
        // Only volumes with a rising trend get one, never "kein Trend".
        #expect(!keller.forecasts.values.contains(.noTrend))
        #expect(keller.fullestVolumeHistory.count == 30)
    }

    @Test func historyAndEventsComeThroughTheStore() throws {
        let now = Int64(Date().timeIntervalSince1970)
        let store = makeStore(MemoryAccountStore(), core: FakeCore())
        store.demoMode = true
        let used = try store.history(nasId: "demo-keller", metric: .volumeUsed, range: .month)
        #expect(used.contains { $0.key == "demo-keller-datavol1" && !$0.points.isEmpty })
        let events = try store.events(nasId: nil, from: now - 30 * 86_400, to: now + 3600)
        #expect(!events.isEmpty)
        #expect(try store.events(nasId: "demo-keller", from: now - 30 * 86_400, to: now + 3600)
            .allSatisfy { $0.nasId == "demo-keller" })

        // Outside demo mode from the core of the accounts.
        defaults.set(false, forKey: NasStore.demoModeKey)
        let core = FakeCore()
        let series = HistorySeries(key: nil, points: [HistoryPoint(at: 100, min: 1, mean: 2, max: 3, count: 4)])
        core.historyPreset.withLock { $0[.cpu] = [series] }
        let event = NasEvent(
            nasId: "a", kind: .volumeFill, subject: "v", level: .warning,
            text: "Volume voll", startedAt: 100, endedAt: nil
        )
        core.presetEvents.withLock { $0["a"] = [event] }
        let live = makeStore(MemoryAccountStore(), core: core)
        #expect(try live.history(nasId: "a", metric: .cpu, range: .day) == [series])
        #expect(try live.history(nasId: "a", metric: .mem, range: .day).isEmpty)
        #expect(try live.events(nasId: "a", from: 0, to: 200) == [event])
    }

    @Test func aStateOfThePlannerReachesTheStoreWithoutAQueryOfItsOwn() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore()
        let store = makeStore(MemoryAccountStore([a]), core: core)
        store.start()
        let listener = try #require(core.listener.withLock { $0 })

        // Forecast and history of the core for the new snapshot.
        let state = Self.state(a, health: .ok)
        let volumes = NasEntry.capacityVolumes(of: try #require(state.snapshot))
        let first = try #require(volumes.first)
        let fullest = try #require(volumes.max { ($0.usedPercent ?? 0) < ($1.usedPercent ?? 0) })
        core.forecastPreset.withLock {
            $0[first.id] = Forecast(daysToFull: 10, fullAt: clock.now + 10 * 86_400, growthPerDay: 1)
        }
        let day: Int64 = 86_400
        core.historyPreset.withLock {
            $0[.volumeUsed] = [HistorySeries(key: fullest.id, points: [
                // Older than 30 days: left out.
                HistoryPoint(at: clock.now - 40 * day, min: 1, mean: 1, max: 1, count: 1),
                HistoryPoint(at: clock.now - 2 * day, min: 50, mean: 50, max: 50, count: 1),
                HistoryPoint(at: clock.now - day, min: 60, mean: 60, max: 60, count: 1),
            ])]
            $0[.volumeTotal] = [HistorySeries(key: fullest.id, points: [
                HistoryPoint(at: clock.now - day, min: 200, mean: 200, max: 200, count: 1),
            ])]
        }

        core.plannerQueried(state)
        #expect(await eventually { store.entries.first?.snapshot != nil })
        #expect(core.calls.refreshed.isEmpty)
        let entry = try #require(store.entries.first)
        #expect(entry.id == id)
        #expect(entry.forecast(for: first.id)
            == .full(days: 10, date: Date(timeIntervalSince1970: TimeInterval(clock.now + 10 * 86_400))))
        let total = try #require(fullest.totalBytes)
        #expect(entry.fullestVolumeHistory == [
            WidgetFillPoint(
                day: Date(timeIntervalSince1970: TimeInterval(clock.now - 2 * day)),
                percent: 50 / Double(total) * 100),
            WidgetFillPoint(day: Date(timeIntervalSince1970: TimeInterval(clock.now - day)), percent: 30),
        ])

        // A state of an id the store does not know changes nothing.
        let before = store.entries.map(\.id)
        listener.stateChanged(state: NasState(nasId: "fremd", snapshot: nil, problem: nil, lastAttemptAt: 1, blocked: false))
        try await Task.sleep(for: .milliseconds(50))
        #expect(store.entries.map(\.id) == before)
    }

    @Test func startHandsTheQueryLogToTheCore() throws {
        let (a, b) = (Self.account("A"), Self.account("B"))
        let (idA, idB) = (a.account.id.uuidString, b.account.id.uuidString)
        let log = NasStore.QueryLog(lastAttempts: [idA: 1_000], blocked: [idB: "Anmeldung abgelehnt"])
        defaults.set(try JSONEncoder().encode(log), forKey: NasStore.queryLogKey)
        defaults.set(15, forKey: NasStore.queryIntervalKey)
        let core = FakeCore()
        let store = makeStore(MemoryAccountStore([a, b]), core: core)
        #expect(core.calls.plannerStarts == 0)
        store.start()
        #expect(core.calls.restoredPolls.sorted { $0.nasId < $1.nasId } == [
            FakeCore.RestoredPoll(nasId: idA, lastAttemptAt: 1_000, blocked: false),
            FakeCore.RestoredPoll(nasId: idB, lastAttemptAt: nil, blocked: true),
        ].sorted { $0.nasId < $1.nasId })
        #expect(core.calls.intervals == [900])
        #expect(core.calls.plannerStarts == 1)
        #expect(core.calls.refreshed.isEmpty)

        store.queryIntervalMinutes = 30
        #expect(core.calls.intervals == [900, 1800])
    }

    @Test func theDemoModeStopsThePlannerAndLeavingItStartsItAgain() throws {
        let core = FakeCore()
        let store = makeStore(MemoryAccountStore([Self.account("Keller")]), core: core)
        store.start()
        #expect(core.calls.plannerStarts == 1)
        store.demoMode = true
        #expect(core.calls.plannerStops == 1)
        #expect(core.listener.withLock { $0 } == nil)
        store.demoMode = false
        #expect(core.calls.plannerStarts == 2)
        #expect(core.listener.withLock { $0 } != nil)

        // Started in demo mode: no planner until it goes off.
        defaults.set(true, forKey: NasStore.demoModeKey)
        let demoCore = FakeCore()
        let demo = makeStore(MemoryAccountStore(), core: demoCore)
        demo.start()
        #expect(demoCore.calls.plannerStarts == 0)
    }

    @Test func anAddedNasCountsAsQueriedForThePlanner() async throws {
        let core = FakeCore()
        let store = await started(MemoryAccountStore(), core: core)
        let new = Self.account("Neu")
        #expect(core.calls.plannerStarts == 1)
        try await store.add(new)
        #expect(core.calls.restoredPolls.last
            == FakeCore.RestoredPoll(nasId: new.account.id.uuidString, lastAttemptAt: clock.now, blocked: false))
        // Held while the core registers the NAS as due, then running again.
        #expect(core.calls.plannerStops == 1)
        #expect(core.calls.plannerStarts == 2)
        #expect(core.listener.withLock { $0 } != nil)
    }

    @Test func aQueryOfThePlannerDuringAnAddIsNotLost() async throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let core = FakeCore()
        let store = makeStore(MemoryAccountStore([a]), core: core)
        store.start()
        var finished = Self.state(a, health: .ok)
        finished.lastAttemptAt = clock.now
        // The planner has no listener while the new NAS is registered.
        core.onAdd.withLock { [finished] in $0 = { core.plannerQueried(finished) } }
        try await store.add(Self.account("Neu"))
        let entry = try #require(store.entries.first { $0.id == id })
        #expect(entry.snapshot != nil)
        #expect(entry.lastAttemptAt == clock.now)
        // The next launch knows the query, so it does not query again.
        let data = try #require(defaults.data(forKey: NasStore.queryLogKey))
        let log = try JSONDecoder().decode(NasStore.QueryLog.self, from: data)
        #expect(log.lastAttempts[id] == clock.now)
    }

    // MARK: - Collector (#124)

    /// A started store with a paired collector that `fake` answers for.
    func startedWithCollector(
        _ keychain: MemoryAccountStore, core: FakeCore, fake: FakeCollector
    ) async -> NasStore {
        let store = NasStore(
            accounts: NasAccounts(keychain: keychain, core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            clock: { [clock] in clock.now },
            collectorStore: MemoryCollectorStore(CollectorSyncTests.collector()),
            listCollectorNas: fake.listNas
        )
        store.start()
        await store.refreshAll()
        await store.collectorTask?.value
        return store
    }

    @Test func anUnreachableCollectorChangesNothingAboutTheNas() async throws {
        let a = Self.account("Keller")
        let fake = FakeCollector()
        fake.failure.withLock { $0 = .Unreachable(detail: "weg") }
        let core = FakeCore(states: [Self.state(a, health: .ok)])

        let store = await startedWithCollector(MemoryAccountStore([a]), core: core, fake: fake)

        #expect(fake.calls.withLock { $0 } == 1)
        #expect(store.collector.error == "Container nicht erreichbar")
        let entry = try #require(store.entries.first)
        #expect(entry.health == .ok)
        #expect(entry.queryError == nil)
        #expect(entry.localError == nil)
        #expect(core.calls.refreshed == [a.account.id.uuidString])
        #expect(store.historyRevision == 0)
    }

    @Test func importedHistoryReloadsTheHistoryScreen() async throws {
        let a = Self.account("Keller")
        let fake = FakeCollector()
        fake.list.withLock { $0 = [CollectorSyncTests.nas("1", fingerprint: a.account.fingerprint)] }
        let core = FakeCore(states: [Self.state(a, health: .ok)])
        core.importResult.withLock { $0 = CollectorImport(until: 7, samples: 12, hourly: 1, daily: 0, backupRuns: 0) }

        let store = await startedWithCollector(MemoryAccountStore([a]), core: core, fake: fake)

        #expect(core.calls.imports.map(\.nasId) == [a.account.id.uuidString])
        #expect(store.historyRevision == 1)
    }

    @Test func theDemoModeDoesNotAskTheCollector() async throws {
        defaults.set(true, forKey: NasStore.demoModeKey)
        let fake = FakeCollector()

        let store = await startedWithCollector(MemoryAccountStore([Self.account("Keller")]), core: FakeCore(), fake: fake)

        #expect(store.collectorTask == nil)
        #expect(fake.calls.withLock { $0 } == 0)
    }

    @Test func removingANasForgetsItsCollectorChoice() async throws {
        let a = Self.account("Keller")
        let fake = FakeCollector()
        let store = await startedWithCollector(MemoryAccountStore([a]), core: FakeCore(), fake: fake)
        try store.collector.assign("1", to: a.account.id)

        try await store.remove(id: a.account.id)

        #expect(store.collector.paired?.assignments == [:])
    }
}
