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
    }

    private let presets: Mutex<[String: NasState]>
    private let registered = Mutex<[String: NasState]>([:])
    private let log = Mutex(Calls())
    /// `addNas` throws `InvalidInput` while set.
    let refuseAdd = Mutex(false)
    /// `updateCredentials` throws `UnknownNas` while set.
    let refuseUpdate = Mutex(false)
    /// `refresh` throws `Internal` while set.
    let failRefresh = Mutex(false)

    init(states: [NasState] = []) {
        presets = Mutex(Dictionary(uniqueKeysWithValues: states.map { ($0.nasId, $0) }))
    }

    var calls: Calls { log.withLock { $0 } }

    func addNas(config: NasConfig, user: String, password: String) throws {
        log.withLock {
            $0.added.append(config.id)
            $0.addedFingerprints.append(config.fingerprint)
        }
        if refuseAdd.withLock({ $0 }) {
            throw FfiError.InvalidInput(detail: "kaputt")
        }
        let empty = NasState(nasId: config.id, snapshot: nil, problem: nil, lastAttemptAt: nil, blocked: false)
        registered.withLock { $0[config.id] = empty }
    }

    func refresh(nasId: String) async throws -> NasState {
        log.withLock { $0.refreshed.append(nasId) }
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

    func testConnection(config: NasConfig, user: String, password: String) async throws -> Snapshot {
        throw FfiError.Internal(detail: "not used")
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

    struct Refused: Error {}

    init(_ stored: [StoredAccount] = []) {
        items.withLock { items in
            for entry in stored {
                items[entry.account.id] = entry
            }
        }
    }

    func loadAll() throws -> [StoredAccount] {
        items.withLock { Array($0.values) }
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
        // Only the given health counts: no fill level or disk temperature
        // that crosses a threshold (#21).
        for index in snapshot.volumes.indices {
            snapshot.volumes[index].usedPercent = nil
        }
        for index in snapshot.disks.indices {
            snapshot.disks[index].temperatureC = nil
        }
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
            oldFingerprint: nil, newFingerprint: nil, httpStatus: nil
        )
    }

    /// Starts a store on `core` with `keychain`.
    func started(_ keychain: MemoryAccountStore, core: FakeCore) async -> NasStore {
        let store = NasStore(
            accounts: NasAccounts(keychain: keychain, core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: nil,
            clock: { [clock] in clock.now }
        )
        await store.start()
        return store
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

        // A second start does nothing.
        await store.start()
        #expect(core.calls.refreshed.count == 2)
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
            demoSource: { _ in [] },
            pollInterval: nil
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

    /// A snapshot with one volume at `fill` percent and one disk at `temp` °C.
    static func snapshot(_ id: String, fill: Double?, temp: Double?) -> Snapshot {
        var snapshot = snapshot(id, health: .ok)
        snapshot.volumes = Array(snapshot.volumes.prefix(1))
        snapshot.disks = Array(snapshot.disks.prefix(1))
        if !snapshot.volumes.isEmpty {
            snapshot.volumes[0].usedPercent = fill
        }
        if !snapshot.disks.isEmpty {
            snapshot.disks[0].temperatureC = temp
        }
        return snapshot
    }

    func makeStore(_ keychain: MemoryAccountStore, core: FakeCore) -> NasStore {
        NasStore(
            accounts: NasAccounts(keychain: keychain, core: core),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: nil,
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

    @Test func theFillThresholdsColourTheNas() throws {
        let a = Self.account("Keller")
        let id = a.account.id.uuidString
        let snapshot = Self.snapshot(id, fill: 85, temp: 30)
        try #require(snapshot.volumes.count == 1)
        let store = makeStore(MemoryAccountStore(), core: FakeCore())
        var entry = NasEntry(id: id, baseUrl: a.account.baseUrl, snapshot: snapshot)
        #expect(entry.health == .warning)
        entry.thresholds.fill = FillThresholds(warning: 90, critical: 95)
        #expect(entry.health == .ok)
        entry.thresholds.fill = FillThresholds(warning: 75, critical: 85)
        #expect(entry.health == .critical)
        // The store hands its thresholds to every entry.
        store.thresholds.fill = FillThresholds(warning: 90, critical: 95)
        store.demoMode = true
        #expect(store.entries.allSatisfy { $0.thresholds.fill.warning == 90 })
    }

    @Test func aHotDiskIsAWarning() throws {
        let id = UUID().uuidString
        let snapshot = Self.snapshot(id, fill: 10, temp: 52)
        try #require(snapshot.disks.count == 1)
        var entry = NasEntry(id: id, baseUrl: "https://nas.demo.invalid", snapshot: snapshot)
        #expect(entry.health == .warning)
        entry.thresholds.diskTempWarningC = 55
        #expect(entry.health == .ok)
        // A missing temperature does not count.
        entry.snapshot = Self.snapshot(id, fill: nil, temp: nil)
        entry.thresholds = .standard
        #expect(entry.health == .ok)
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

    @Test func theStoreThresholdsColourAndSortRealNas() async throws {
        let (a, b) = (Self.account("Anton"), Self.account("Berta"))
        let core = FakeCore(states: [
            NasState(
                nasId: a.account.id.uuidString,
                snapshot: Self.snapshot(a.account.id.uuidString, fill: 10, temp: 30),
                problem: nil, lastAttemptAt: 100, blocked: false
            ),
            NasState(
                nasId: b.account.id.uuidString,
                snapshot: Self.snapshot(b.account.id.uuidString, fill: 85, temp: 30),
                problem: nil, lastAttemptAt: 100, blocked: false
            ),
        ])
        let store = await started(MemoryAccountStore([a, b]), core: core)
        #expect(store.entries.map(\.displayName) == ["Berta", "Anton"])
        #expect(store.entries.map(\.health) == [.warning, .ok])

        store.thresholds.fill = FillThresholds(warning: 90, critical: 95)
        #expect(store.entries.map(\.displayName) == ["Anton", "Berta"])
        #expect(store.entries.map(\.health) == [.ok, .ok])
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
}
