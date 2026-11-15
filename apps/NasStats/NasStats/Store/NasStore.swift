import Foundation
import Observation

/// The state of every NAS for all screens, the menu bar and later the
/// widgets: queries the NAS of `NasAccounts` and holds the last state per
/// NAS, worst first.
///
/// History, forecast and events follow with #67; the planner of the core
/// replaces the simple poll loop then.
@MainActor
@Observable
final class NasStore {
    /// Every NAS, sorted by `NasEntry.ordered`. In demo mode the three demo
    /// NAS instead.
    private(set) var entries: [NasEntry] = []

    /// Shows the demo data set (#22) instead of the stored NAS; no NAS is
    /// queried meanwhile. Saved under `demoMode` in the defaults.
    var demoMode: Bool {
        didSet {
            guard demoMode != oldValue else { return }
            defaults.set(demoMode, forKey: Self.demoModeKey)
            rebuild()
            if !demoMode, started {
                Task { await refreshAll() }
            }
        }
    }

    /// Called after every new state with the sorted entries. #23 writes the
    /// widget snapshot here and reloads the widgets.
    @ObservationIgnored var onUpdate: (@MainActor ([NasEntry]) -> Void)?

    static let demoModeKey = "demoMode"
    static let queryLogKey = "queryLog"
    /// Same as `MIN_INTERVAL_SECS` of the core.
    static let minInterval: Int64 = 300

    /// What the store keeps across launches, because the core holds its
    /// block and the time of the last query only in memory: without it,
    /// every launch would query at once and retry a refused login.
    struct QueryLog: Codable, Equatable {
        /// Start of the last query per NAS id, Unix seconds UTC.
        var lastAttempts: [String: Int64] = [:]
        /// NAS the core blocked (refused login, second factor, changed
        /// certificate) with the reason; not queried until the user acts.
        var blocked: [String: String] = [:]
    }

    /// List, passwords and registration with the core (#14).
    @ObservationIgnored let accounts: NasAccounts
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let demoSource: (Int64) -> [Snapshot]
    @ObservationIgnored private let pollInterval: Duration?
    @ObservationIgnored private let clock: () -> Int64

    @ObservationIgnored private var queryErrors: [String: String] = [:]
    @ObservationIgnored private var log: QueryLog
    /// Snapshot of the test query of the add dialog, until the core has one.
    @ObservationIgnored private var testSnapshots: [String: Snapshot] = [:]
    @ObservationIgnored private var started = false
    /// Lives as long as the store; ends once the store is gone.
    @ObservationIgnored private var pollTask: Task<Void, Never>?

    /// - Parameter pollInterval: Time between two rounds of queries; `nil`
    ///   queries only on `start`, `add`, `updateCredentials` and explicit
    ///   `refreshAll` calls.
    init(
        accounts: NasAccounts,
        defaults: UserDefaults,
        demoSource: @escaping (Int64) -> [Snapshot],
        pollInterval: Duration?,
        clock: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.clock = clock
        self.accounts = accounts
        self.defaults = defaults
        self.demoSource = demoSource
        self.pollInterval = pollInterval
        self.log = defaults.data(forKey: Self.queryLogKey)
            .flatMap { try? JSONDecoder().decode(QueryLog.self, from: $0) } ?? QueryLog()
        self.demoMode = defaults.bool(forKey: Self.demoModeKey)
        // The list shows the stored NAS at once, before the first query.
        rebuild()
    }

    /// The store of the app: the NAS from the keychain, the demo data of the
    /// core, the demo switch in the defaults of the App Group and a query
    /// every 5 minutes.
    static func live() -> NasStore {
        NasStore(
            accounts: NasAccounts(),
            defaults: UserDefaults(suiteName: AppGroup.id) ?? .standard,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: .seconds(300)
        )
    }

    /// The core of the accounts, e.g. for the test query of the add dialog.
    var core: any NasCoreProtocol { accounts.core }

    /// Set when the keychain could not be read.
    var loadError: String? { accounts.loadError }

    // MARK: - Start and queries

    /// Queries the stored NAS (see `refreshAll`), then every
    /// `pollInterval`. Runs once, later calls do nothing.
    func start() async {
        guard !started else { return }
        started = true
        await refreshAll()
        guard let pollInterval else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: pollInterval)
                guard !Task.isCancelled, let self else { return }
                await self.refreshAll()
            }
        }
    }

    /// Queries every registered NAS in parallel, except those blocked or
    /// queried less than 5 minutes ago, also in an earlier launch or by the
    /// test query of the add dialog. Does nothing in demo mode.
    func refreshAll() async {
        guard !demoMode else { return }
        await query(queryable())
    }

    // MARK: - Changing the list

    /// Stores a NAS after the test query of the add dialog. That query
    /// counts as its first: its snapshot shows until the core has its own,
    /// and the next query follows with the poll round after 5 minutes.
    /// Throws when the keychain or the core refuses it; nothing changes then.
    func add(_ stored: StoredAccount, snapshot: Snapshot? = nil) async throws {
        try accounts.add(stored)
        let id = stored.account.id.uuidString
        queryErrors[id] = nil
        testSnapshots[id] = snapshot
        log.blocked[id] = nil
        log.lastAttempts[id] = clock()
        saveLog()
        rebuild()
    }

    /// Removes a NAS from the keychain, the core and the list.
    func remove(id: UUID) async throws {
        try await accounts.remove(id: id)
        queryErrors[id.uuidString] = nil
        testSnapshots[id.uuidString] = nil
        log.blocked[id.uuidString] = nil
        log.lastAttempts[id.uuidString] = nil
        saveLog()
        rebuild()
    }

    /// New user name and password for a stored NAS, e.g. after a refused
    /// login; lifts a block and queries again. When the core refuses them,
    /// the old credentials stay stored.
    func updateCredentials(id: UUID, user: String, password: String) async throws {
        do {
            try await accounts.updateCredentials(id: id, user: user, password: password)
        } catch {
            // A refused registration is now in the entry.
            rebuild()
            throw error
        }
        queryErrors[id.uuidString] = nil
        log.blocked[id.uuidString] = nil
        saveLog()
        rebuild()
        if !demoMode {
            await query([id])
        }
    }

    // MARK: - Internals

    /// Registered NAS that are not blocked and were not queried in the
    /// last 5 minutes.
    private func queryable() -> [UUID] {
        let now = clock()
        return accounts.accounts.map(\.id).filter { id in
            let key = id.uuidString
            guard accounts.isRegistered(id), log.blocked[key] == nil else { return false }
            guard let last = log.lastAttempts[key] else { return true }
            return now < last || now - last >= Self.minInterval
        }
    }

    private func saveLog() {
        guard let data = try? JSONEncoder().encode(log) else { return }
        defaults.set(data, forKey: Self.queryLogKey)
    }

    /// Queries the NAS in parallel. The time of the attempt is saved
    /// before the request and every answer as soon as it comes, so a quit
    /// in the middle of a round neither repeats a query nor loses a block.
    private func query(_ ids: [UUID]) async {
        guard !ids.isEmpty else { return }
        let now = clock()
        for id in ids {
            log.lastAttempts[id.uuidString] = now
        }
        saveLog()
        let core = accounts.core
        await withTaskGroup(of: (UUID, NasState?, String?).self) { group in
            for id in ids {
                group.addTask {
                    do {
                        return (id, try await core.refresh(nasId: id.uuidString), nil)
                    } catch {
                        return (id, nil, connectionErrorText(error))
                    }
                }
            }
            for await (id, state, error) in group {
                record(id, state: state, error: error)
            }
        }
    }

    /// Takes in the answer of one query.
    private func record(_ id: UUID, state: NasState?, error: String?) {
        // A NAS removed while the query ran stays out.
        guard accounts.isRegistered(id) else { return }
        let key = id.uuidString
        queryErrors[key] = error
        if let state {
            if let last = state.lastAttemptAt {
                log.lastAttempts[key] = max(last, log.lastAttempts[key] ?? last)
            }
            if state.blocked {
                log.blocked[key] = state.problem?.message ?? "Gesperrt"
            }
            if state.snapshot != nil {
                testSnapshots[key] = nil
            }
        }
        saveLog()
        rebuild()
    }

    /// Builds `entries` from the accounts and the states of the core (or
    /// from the demo data), sorts them and reports them.
    private func rebuild() {
        if demoMode {
            let now = Int64(Date().timeIntervalSince1970)
            entries = demoSource(now).map { snapshot in
                NasEntry(
                    id: snapshot.nas.id,
                    name: snapshot.nas.name,
                    baseUrl: snapshot.nas.baseUrl,
                    provider: snapshot.nas.provider,
                    snapshot: snapshot
                )
            }
        } else {
            let states = Dictionary(
                accounts.core.states().map { ($0.nasId, $0) },
                uniquingKeysWith: { first, _ in first }
            )
            entries = accounts.accounts.map { account in
                let id = account.id.uuidString
                let state = accounts.isRegistered(account.id) ? states[id] : nil
                return NasEntry(
                    id: id,
                    name: account.name.isEmpty ? nil : account.name,
                    baseUrl: account.baseUrl,
                    model: account.model,
                    provider: account.config?.provider,
                    snapshot: state?.snapshot ?? testSnapshots[id],
                    problem: state?.problem,
                    lastAttemptAt: state?.lastAttemptAt ?? log.lastAttempts[id],
                    // A block of an earlier launch holds until the user acts.
                    blocked: (state?.blocked ?? false) || log.blocked[id] != nil,
                    // After a restart the reason of a block comes from the log.
                    localError: accounts.registrationErrors[account.id]
                        ?? (state?.problem == nil ? log.blocked[id] : nil),
                    queryError: queryErrors[id]
                )
            }
        }
        entries.sort(by: NasEntry.ordered)
        onUpdate?(entries)
    }
}
