import Foundation
import Synchronization
import Testing

/// Core double: answers with preset states and records every call.
final class FakeCore: NasCoreProtocol {
    struct Calls {
        var added: [String] = []
        var removed: [String] = []
        var refreshed: [String] = []
        var updated: [String] = []
    }

    private let presets: Mutex<[String: NasState]>
    private let registered = Mutex<[String: NasState]>([:])
    private let log = Mutex(Calls())

    init(states: [NasState] = []) {
        presets = Mutex(Dictionary(uniqueKeysWithValues: states.map { ($0.nasId, $0) }))
    }

    var calls: Calls { log.withLock { $0 } }

    func addNas(config: NasConfig, user: String, password: String) throws {
        log.withLock { $0.added.append(config.id) }
        let empty = NasState(nasId: config.id, snapshot: nil, problem: nil, lastAttemptAt: nil, blocked: false)
        registered.withLock { $0[config.id] = empty }
    }

    func refresh(nasId: String) async throws -> NasState {
        log.withLock { $0.refreshed.append(nasId) }
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
        log.withLock { $0.updated.append(nasId) }
    }
}

/// Password store that refuses to save.
final class FailingCredentialStore: CredentialStore {
    struct Refused: Error {}

    func load(nasId: String) throws -> Credentials? { nil }
    func save(_ credentials: Credentials, nasId: String) throws { throw Refused() }
    func delete(nasId: String) throws {}
}

@MainActor
struct NasStoreTests {
    let defaults: UserDefaults
    let credentials = InMemoryCredentialStore()
    let login = Credentials(user: "monitor", password: "demo")

    init() throws {
        defaults = try #require(UserDefaults(suiteName: "nasstats-tests-\(UUID().uuidString)"))
    }

    static func nas(_ id: String, name: String) -> StoredNas {
        StoredNas(
            id: id,
            provider: .qnap,
            baseUrl: "https://\(id).demo.invalid",
            fingerprint: String(repeating: "AB", count: 32),
            name: name
        )
    }

    static func snapshot(_ id: String, health: Health?) -> Snapshot {
        var snapshot = demoSnapshot()
        snapshot.nas.id = id
        snapshot.demo = false
        snapshot.worstHealth = health
        return snapshot
    }

    static func state(
        _ id: String,
        health: Health?,
        problem: NasProblem? = nil,
        blocked: Bool = false
    ) -> NasState {
        NasState(
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

    /// Saves `list` with passwords and starts a store on `core`.
    func started(
        _ list: [StoredNas],
        core: FakeCore,
        withPasswords: Bool = true
    ) async throws -> NasStore {
        NasList(defaults: defaults).save(list)
        if withPasswords {
            for nas in list {
                try credentials.save(login, nasId: nas.id)
            }
        }
        let store = NasStore(
            core: core,
            credentials: credentials,
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: nil
        )
        await store.start()
        return store
    }

    @Test func sortsCriticalWarningOkUnknown() async throws {
        let core = FakeCore(states: [
            Self.state("a", health: .ok),
            Self.state("b", health: nil),
            Self.state("c", health: .warning),
            Self.state("d", health: .critical),
        ])
        let store = try await started(
            [
                Self.nas("a", name: "Anton"), Self.nas("b", name: "Berta"),
                Self.nas("c", name: "Cäsar"), Self.nas("d", name: "Dora"),
            ],
            core: core
        )
        #expect(store.entries.map(\.id) == ["d", "c", "a", "b"])
        #expect(store.entries.map(\.health) == [.critical, .warning, .ok, .unknown])
    }

    @Test func sameHealthSortsByName() async throws {
        let core = FakeCore(states: [Self.state("x", health: .ok), Self.state("y", health: .ok)])
        let store = try await started([Self.nas("x", name: "Zeta"), Self.nas("y", name: "alpha")], core: core)
        #expect(store.entries.map(\.displayName) == ["alpha", "Zeta"])
    }

    @Test func errorKeepsTheSnapshotAndLiftsToWarning() async throws {
        let core = FakeCore(states: [
            Self.state("a", health: .ok, problem: Self.problem(.unreachable, stops: false)),
        ])
        let store = try await started([Self.nas("a", name: "Keller")], core: core)
        let entry = try #require(store.entries.first)
        #expect(entry.snapshot != nil)
        #expect(entry.observedAt != nil)
        #expect(entry.problem?.kind == .unreachable)
        #expect(entry.health == .warning)
    }

    @Test func blockedNasIsCritical() async throws {
        let core = FakeCore(states: [
            Self.state("a", health: .ok, problem: Self.problem(.loginFailed, stops: true), blocked: true),
        ])
        let store = try await started([Self.nas("a", name: "Keller")], core: core)
        #expect(store.entries.first?.health == .critical)
    }

    @Test func missingPasswordIsCriticalAndNotRegistered() async throws {
        let core = FakeCore()
        let store = try await started([Self.nas("a", name: "Keller")], core: core, withPasswords: false)
        let entry = try #require(store.entries.first)
        #expect(entry.localError == NasStore.missingPassword)
        #expect(entry.health == .critical)
        #expect(core.calls.added.isEmpty)
        #expect(core.calls.refreshed.isEmpty)
    }

    @Test func startRegistersAndQueriesSavedNas() async throws {
        let core = FakeCore()
        let store = try await started([Self.nas("a", name: "A"), Self.nas("b", name: "B")], core: core)
        #expect(core.calls.added.sorted() == ["a", "b"])
        #expect(core.calls.refreshed.sorted() == ["a", "b"])
        #expect(store.entries.count == 2)

        // A second start does nothing.
        await store.start()
        #expect(core.calls.added.count == 2)
    }

    @Test func addAndRemoveChangeListAndPasswords() async throws {
        let core = FakeCore()
        let store = try await started([], core: core)
        let nas = Self.nas("new", name: "Neu")

        try await store.add(nas, credentials: login)
        #expect(NasList(defaults: defaults).load() == [nas])
        #expect(try credentials.load(nasId: "new") == login)
        #expect(core.calls.added == ["new"])
        #expect(core.calls.refreshed == ["new"])
        #expect(store.entries.map(\.id) == ["new"])

        await store.remove(id: "new")
        #expect(NasList(defaults: defaults).load().isEmpty)
        #expect(try credentials.load(nasId: "new") == nil)
        #expect(core.calls.removed == ["new"])
        #expect(store.entries.isEmpty)
    }

    @Test func addKeepsTheListWhenThePasswordCannotBeSaved() async throws {
        let core = FakeCore()
        let store = NasStore(
            core: core,
            credentials: FailingCredentialStore(),
            defaults: defaults,
            demoSource: { _ in [] },
            pollInterval: nil
        )
        await store.start()
        await #expect(throws: FailingCredentialStore.Refused.self) {
            try await store.add(Self.nas("new", name: "Neu"), credentials: login)
        }
        #expect(NasList(defaults: defaults).load().isEmpty)
        #expect(core.calls.removed == ["new"])
        #expect(core.states().isEmpty)
        #expect(store.entries.isEmpty)
    }

    @Test func updateCredentialsRegistersANasWithoutPassword() async throws {
        let core = FakeCore()
        let store = try await started([Self.nas("a", name: "Keller")], core: core, withPasswords: false)
        try await store.updateCredentials(id: "a", credentials: login)
        #expect(core.calls.added == ["a"])
        #expect(core.calls.refreshed == ["a"])
        #expect(try credentials.load(nasId: "a") == login)
        #expect(store.entries.first?.localError == nil)

        try await store.updateCredentials(id: "a", credentials: login)
        #expect(core.calls.updated == ["a"])
    }

    @Test func demoModeShowsTheThreeDemoNasWithoutQueries() async throws {
        defaults.set(true, forKey: NasStore.demoModeKey)
        let core = FakeCore()
        let store = try await started([Self.nas("a", name: "Keller echt")], core: core)
        // Registered, but not queried.
        #expect(core.calls.added == ["a"])
        await store.refreshAll()
        #expect(core.calls.refreshed.isEmpty)

        #expect(store.entries.map(\.id).sorted() == ["demo-buero", "demo-homelab", "demo-keller"])
        #expect(store.entries.allSatisfy { $0.snapshot?.demo == true })

        store.demoMode = false
        #expect(!defaults.bool(forKey: NasStore.demoModeKey))
        #expect(store.entries.map(\.id) == ["a"])
    }

    @Test func onUpdateFollowsEveryNewState() async throws {
        let core = FakeCore(states: [Self.state("a", health: .ok)])
        let store = try await started([Self.nas("a", name: "Keller")], core: core)
        var reported: [[String]] = []
        store.onUpdate = { reported.append($0.map(\.id)) }

        await store.refreshAll()
        #expect(reported == [["a"]])

        store.demoMode = true
        #expect(reported.count == 2)
        #expect(reported.last?.count == 3)
    }

    @Test func listSkipsUnknownProvidersAndBrokenJson() throws {
        let list = NasList(defaults: defaults)
        let json = """
        [{"id":"a","provider":"qnap","baseUrl":"https://a","fingerprint":"AB"},
         {"id":"b","provider":"omv","baseUrl":"https://b","fingerprint":"AB"}]
        """
        defaults.set(Data(json.utf8), forKey: NasList.key)
        #expect(list.load().map(\.id) == ["a"])
        #expect(list.load().first?.name == nil)

        defaults.set(Data("kaputt".utf8), forKey: NasList.key)
        #expect(list.load().isEmpty)
    }
}
