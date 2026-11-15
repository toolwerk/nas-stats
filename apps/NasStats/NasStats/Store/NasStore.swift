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

    /// List, passwords and registration with the core (#14).
    @ObservationIgnored let accounts: NasAccounts
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let demoSource: (Int64) -> [Snapshot]
    @ObservationIgnored private let pollInterval: Duration?

    @ObservationIgnored private var queryErrors: [String: String] = [:]
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
        self.demoMode = defaults.bool(forKey: Self.demoModeKey)
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

    /// Shows the stored NAS and queries them; then every `pollInterval`.
    /// Runs once, later calls do nothing.
    func start() async {
        guard !started else { return }
        started = true
        rebuild()
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

    /// Queries every registered NAS in parallel. The core skips a NAS that
    /// is blocked or was queried less than 5 minutes ago. Does nothing in
    /// demo mode.
    func refreshAll() async {
        guard !demoMode else { return }
        await query(accounts.accounts.filter { accounts.isRegistered($0.id) }.map(\.id))
    }

    // MARK: - Changing the list

    /// Stores a NAS after the test query of the add dialog and queries it.
    /// Throws when the keychain or the core refuses it; nothing changes then.
    func add(_ stored: StoredAccount) async throws {
        try accounts.add(stored)
        queryErrors[stored.account.id.uuidString] = nil
        rebuild()
        if !demoMode {
            await query([stored.account.id])
        }
    }

    /// Removes a NAS from the keychain, the core and the list.
    func remove(id: UUID) async throws {
        try await accounts.remove(id: id)
        queryErrors[id.uuidString] = nil
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
        rebuild()
        if !demoMode {
            await query([id])
        }
    }

    // MARK: - Internals

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
        for (id, error) in results where accounts.isRegistered(id) {
            queryErrors[id.uuidString] = error
        }
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
                    snapshot: state?.snapshot,
                    problem: state?.problem,
                    lastAttemptAt: state?.lastAttemptAt,
                    blocked: state?.blocked ?? false,
                    localError: accounts.registrationErrors[account.id],
                    queryError: queryErrors[id]
                )
            }
        }
        entries.sort(by: NasEntry.ordered)
        onUpdate?(entries)
    }
}
