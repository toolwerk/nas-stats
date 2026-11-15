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
        /// certificate); not queried until the user acts.
        var blocked: Set<String> = []
    }

    /// List, passwords and registration with the core (#14).
    @ObservationIgnored let accounts: NasAccounts
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let demoSource: (Int64) -> [Snapshot]
    @ObservationIgnored private let pollInterval: Duration?

    @ObservationIgnored private var queryErrors: [String: String] = [:]
    @ObservationIgnored private var log: QueryLog
    /// Snapshot of the test query of the add dialog, until the core has one.
    @ObservationIgnored private var testSnapshots: [String: Snapshot] = [:]
    @ObservationIgnored private var started = false
    /// Lives as long as the store; ends once the store is gone.
    @ObservationIgnored private var pollTask: Task<Void, Never>?

    /// - Parameter pollInterval: Time between two rounds of queries; `nil`
    ///   queries only on `start`, `add`, `updatePassword` and explicit
    ///   `refreshAll` calls.
    init(
        accounts: NasAccounts,
        defaults: UserDefaults,
        demoSource: @escaping (Int64) -> [Snapshot],
        pollInterval: Duration?
    ) {
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

    /// Queries the stored NAS, except those blocked or queried less than
    /// 5 minutes ago in an earlier launch; then every `pollInterval`. Runs
    /// once, later calls do nothing.
    func start() async {
        guard !started else { return }
        started = true
        if !demoMode {
            let now = Self.now()
            await query(queryable().filter { id in
                guard let last = log.lastAttempts[id.uuidString] else { return true }
                return now < last || now - last >= Self.minInterval
            })
        }
        guard let pollInterval else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: pollInterval)
                guard !Task.isCancelled, let self else { return }
                await self.refreshAll()
            }
        }
    }

    /// Queries every registered NAS in parallel. The core skips a NAS that
    /// is blocked or was queried less than 5 minutes ago. Does nothing in
    /// demo mode.
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
        log.blocked.remove(id)
        log.lastAttempts[id] = Self.now()
        saveLog()
        rebuild()
    }

    /// Removes a NAS from the keychain, the core and the list.
    func remove(id: UUID) async throws {
        try await accounts.remove(id: id)
        queryErrors[id.uuidString] = nil
        testSnapshots[id.uuidString] = nil
        log.blocked.remove(id.uuidString)
        log.lastAttempts[id.uuidString] = nil
        saveLog()
        rebuild()
    }

    /// New password for a stored NAS, e.g. after a refused login; lifts a
    /// block and queries again. When the core refuses it, the old password
    /// stays stored.
    func updatePassword(id: UUID, password: String) async throws {
        do {
            try await accounts.updatePassword(id: id, password: password)
        } catch {
            // A refused registration is now in the entry.
            rebuild()
            throw error
        }
        queryErrors[id.uuidString] = nil
        log.blocked.remove(id.uuidString)
        saveLog()
        rebuild()
        if !demoMode {
            await query([id])
        }
    }

    // MARK: - Internals

    /// Registered NAS that were not blocked in an earlier launch.
    private func queryable() -> [UUID] {
        accounts.accounts
            .filter { accounts.isRegistered($0.id) && !log.blocked.contains($0.id.uuidString) }
            .map(\.id)
    }

    private static func now() -> Int64 {
        Int64(Date().timeIntervalSince1970)
    }

    private func saveLog() {
        guard let data = try? JSONEncoder().encode(log) else { return }
        defaults.set(data, forKey: Self.queryLogKey)
    }

    private func query(_ ids: [UUID]) async {
        guard !ids.isEmpty else { return }
        let core = accounts.core
        let results = await withTaskGroup(of: (UUID, String?).self) { group in
            for id in ids {
                group.addTask {
                    do {
                        _ = try await core.refresh(nasId: id.uuidString)
                        return (id, nil)
                    } catch {
                        return (id, connectionErrorText(error))
                    }
                }
            }
            var results: [(UUID, String?)] = []
            for await result in group {
                results.append(result)
            }
            return results
        }
        // A NAS removed while the query ran stays out.
        let states = Dictionary(
            accounts.core.states().map { ($0.nasId, $0) },
            uniquingKeysWith: { first, _ in first }
        )
        for (id, error) in results where accounts.isRegistered(id) {
            let key = id.uuidString
            queryErrors[key] = error
            if let state = states[key] {
                if let last = state.lastAttemptAt {
                    log.lastAttempts[key] = last
                }
                if state.blocked {
                    log.blocked.insert(key)
                }
                if state.snapshot != nil {
                    testSnapshots[key] = nil
                }
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
                    snapshot: state?.snapshot ?? testSnapshots[id],
                    problem: state?.problem,
                    lastAttemptAt: state?.lastAttemptAt ?? log.lastAttempts[id],
                    // A block of an earlier launch holds until the user acts.
                    blocked: (state?.blocked ?? false) || log.blocked.contains(id),
                    localError: accounts.registrationErrors[account.id],
                    queryError: queryErrors[id]
                )
            }
        }
        entries.sort(by: NasEntry.ordered)
        onUpdate?(entries)
    }
}
