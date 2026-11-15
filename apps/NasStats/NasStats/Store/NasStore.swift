import Foundation
import Observation

/// The state of every NAS for all screens, the menu bar and later the
/// widgets: keeps the NAS list, registers it with the core, queries it and
/// holds the last state per NAS, worst first.
///
/// History, forecast and events follow with #67; the planner of the core
/// replaces the simple poll loop then.
@MainActor
@Observable
final class NasStore {
    /// Every NAS, sorted by `NasEntry.ordered`. In demo mode the three demo
    /// NAS instead.
    private(set) var entries: [NasEntry] = []

    /// Shows the demo data set (#22) instead of the saved NAS; no NAS is
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
    static let missingPassword = "Passwort fehlt"

    @ObservationIgnored private let core: any NasCoreProtocol
    @ObservationIgnored private let credentials: any CredentialStore
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let list: NasList
    @ObservationIgnored private let demoSource: (Int64) -> [Snapshot]
    @ObservationIgnored private let pollInterval: Duration?

    @ObservationIgnored private var stored: [StoredNas] = []
    /// NAS the core knows; the others lack a password or were refused.
    @ObservationIgnored private var registered: Set<String> = []
    @ObservationIgnored private var localErrors: [String: String] = [:]
    @ObservationIgnored private var started = false
    /// Lives as long as the store; ends once the store is gone.
    @ObservationIgnored private var pollTask: Task<Void, Never>?

    /// - Parameter pollInterval: Time between two rounds of queries; `nil`
    ///   queries only on `start`, `add` and explicit `refreshAll` calls.
    init(
        core: any NasCoreProtocol,
        credentials: any CredentialStore,
        defaults: UserDefaults,
        demoSource: @escaping (Int64) -> [Snapshot],
        pollInterval: Duration?
    ) {
        self.core = core
        self.credentials = credentials
        self.defaults = defaults
        self.list = NasList(defaults: defaults)
        self.demoSource = demoSource
        self.pollInterval = pollInterval
        self.demoMode = defaults.bool(forKey: Self.demoModeKey)
    }

    /// The store of the app: the Rust core, passwords in memory (keychain
    /// with #14), the defaults of the App Group, the demo data of the core
    /// and a query every 5 minutes.
    static func live() -> NasStore {
        NasStore(
            core: NasCore(),
            credentials: InMemoryCredentialStore(),
            defaults: UserDefaults(suiteName: AppGroup.id) ?? .standard,
            demoSource: { demoSnapshots(now: $0) },
            pollInterval: .seconds(300)
        )
    }

    // MARK: - Start and queries

    /// Loads the NAS list, registers every NAS with the core and queries
    /// them; then every `pollInterval`. Runs once, later calls do nothing.
    func start() async {
        guard !started else { return }
        started = true
        stored = list.load()
        for nas in stored {
            register(nas)
        }
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
        await query(stored.map(\.id).filter(registered.contains))
    }

    // MARK: - Changing the list

    /// Saves a new NAS, or replaces one with the same id, and queries it.
    /// Throws when the core refuses it or the password cannot be saved;
    /// the list stays unchanged then.
    func add(_ nas: StoredNas, credentials new: Credentials) async throws {
        let known = registered.contains(nas.id)
        try core.addNas(config: nas.config, user: new.user, password: new.password)
        do {
            try credentials.save(new, nasId: nas.id)
        } catch {
            // A new NAS leaves the core again; a replaced one keeps the new
            // config, which the next start overwrites from the list.
            if !known {
                try? await core.removeNas(nasId: nas.id)
            }
            throw error
        }
        if let index = stored.firstIndex(where: { $0.id == nas.id }) {
            stored[index] = nas
        } else {
            stored.append(nas)
        }
        list.save(stored)
        registered.insert(nas.id)
        localErrors[nas.id] = nil
        rebuild()
        if !demoMode {
            await query([nas.id])
        }
    }

    /// Removes a NAS from the core, the password store and the list.
    func remove(id: String) async {
        // The core only throws `UnknownNas` (not registered) or an internal
        // error; the NAS leaves the list either way.
        try? await core.removeNas(nasId: id)
        try? credentials.delete(nasId: id)
        stored.removeAll { $0.id == id }
        list.save(stored)
        registered.remove(id)
        localErrors[id] = nil
        rebuild()
    }

    /// New user name or password for a saved NAS, e.g. after a refused
    /// login or "Passwort fehlt"; lifts a block and queries again.
    func updateCredentials(id: String, credentials new: Credentials) async throws {
        guard let nas = stored.first(where: { $0.id == id }) else {
            throw FfiError.UnknownNas(id: id)
        }
        try credentials.save(new, nasId: id)
        if registered.contains(id) {
            try await core.updateCredentials(nasId: id, user: new.user, password: new.password)
        } else {
            // Not registered at the start (password missing or refused).
            try core.addNas(config: nas.config, user: new.user, password: new.password)
            registered.insert(id)
        }
        localErrors[id] = nil
        rebuild()
        if !demoMode {
            await query([id])
        }
    }

    // MARK: - Internals

    /// Registers a saved NAS with the core, or notes why it cannot be.
    private func register(_ nas: StoredNas) {
        do {
            guard let saved = try credentials.load(nasId: nas.id) else {
                localErrors[nas.id] = Self.missingPassword
                return
            }
            try core.addNas(config: nas.config, user: saved.user, password: saved.password)
            registered.insert(nas.id)
            localErrors[nas.id] = nil
        } catch {
            localErrors[nas.id] = Self.message(for: error)
        }
    }

    private func query(_ ids: [String]) async {
        guard !ids.isEmpty else { return }
        let core = self.core
        let results = await withTaskGroup(of: (String, String?).self) { group in
            for id in ids {
                group.addTask {
                    do {
                        _ = try await core.refresh(nasId: id)
                        return (id, nil)
                    } catch {
                        return (id, Self.message(for: error))
                    }
                }
            }
            var results: [(String, String?)] = []
            for await result in group {
                results.append(result)
            }
            return results
        }
        // A NAS removed while the query ran stays out.
        for (id, error) in results where registered.contains(id) {
            localErrors[id] = error
        }
        rebuild()
    }

    /// Builds `entries` from the list and the states of the core (or from
    /// the demo data), sorts them and reports them.
    private func rebuild() {
        if demoMode {
            let now = Int64(Date().timeIntervalSince1970)
            entries = demoSource(now).map { snapshot in
                NasEntry(
                    stored: StoredNas(
                        id: snapshot.nas.id,
                        provider: snapshot.nas.provider,
                        baseUrl: snapshot.nas.baseUrl,
                        fingerprint: "",
                        name: snapshot.nas.name
                    ),
                    snapshot: snapshot
                )
            }
        } else {
            let states = Dictionary(
                core.states().map { ($0.nasId, $0) },
                uniquingKeysWith: { first, _ in first }
            )
            entries = stored.map { nas in
                let state = registered.contains(nas.id) ? states[nas.id] : nil
                return NasEntry(
                    stored: nas,
                    snapshot: state?.snapshot,
                    problem: state?.problem,
                    lastAttemptAt: state?.lastAttemptAt,
                    blocked: state?.blocked ?? false,
                    localError: localErrors[nas.id]
                )
            }
        }
        entries.sort(by: NasEntry.ordered)
        onUpdate?(entries)
    }

    /// Readable text of an error from the core or the password store.
    nonisolated static func message(for error: any Error) -> String {
        guard let error = error as? FfiError else {
            return error.localizedDescription
        }
        return switch error {
        case let .Unreachable(detail): "NAS nicht erreichbar: \(detail)"
        case .Timeout: "Zeitüberschreitung bei der Abfrage"
        case .LoginFailed: "Anmeldung fehlgeschlagen"
        case .SecondFactorRequired: "NAS verlangt einen zweiten Faktor"
        case .PermissionDenied: "Konto hat keine Rechte für diese Abfrage"
        case .CertificateChanged: "Zertifikat des NAS hat sich geändert"
        case let .Parse(detail): "Unerwartete Antwort: \(detail)"
        case let .Http(status): "HTTP-Status \(status)"
        case let .UnknownNas(id): "Kein NAS mit der Kennung \(id)"
        case .Unsupported: "Anbieter wird noch nicht unterstützt"
        case let .InvalidInput(detail): "Ungültige Angabe: \(detail)"
        case let .Internal(detail): "Interner Fehler: \(detail)"
        }
    }
}
