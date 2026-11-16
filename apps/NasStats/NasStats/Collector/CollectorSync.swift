import Foundation
import Observation
import os

/// Pairing with the collector on the NAS (#46) and the import of its
/// history (#124): after a round of queries, at most once per query
/// interval, the history of every assigned NAS since the last import comes
/// into the archive of the core. Apart from the queries of the NAS: an
/// error of the collector stays here and changes nothing about them.
@MainActor
@Observable
final class CollectorSync {
    /// Lists the NAS of a collector: base URL, fingerprint, key.
    typealias ListNas = @Sendable (String, String, String) async throws -> [CollectorNas]

    /// One NAS of the collector taken over into a NAS of the app.
    struct Assignment: Equatable {
        var appNasId: UUID
        /// Found by the fingerprint, not chosen by the user.
        var automatic: Bool
    }

    /// What this device keeps across launches under `collectorSync` in the
    /// defaults of the App Group; each device has its own history.
    struct SyncState: Codable, Equatable {
        /// Start of the last sync, Unix seconds UTC.
        var lastSyncAt: Int64?
        /// End of the last sync that reached the collector.
        var lastSuccessAt: Int64?
        /// Why the collector refused us (key, certificate); no automatic
        /// sync until the user pairs again or syncs by hand.
        var stopped: String?
        /// Per NAS of the app (uuid string): from which NAS of the collector
        /// it took over and up to when.
        var progress: [String: Progress] = [:]
    }

    struct Progress: Codable, Equatable {
        var collectorNasId: String
        var until: Int64
    }

    static let stateKey = "collectorSync"

    /// The paired collector; `nil` without one.
    private(set) var paired: StoredCollector?
    /// The NAS of the collector at the last sync.
    private(set) var nas: [CollectorNas] = []
    /// Error of the collector at the last sync, or why it stopped.
    private(set) var error: String?
    /// Error of the import per NAS of the collector at the last sync.
    private(set) var nasErrors: [String: String] = [:]
    /// A sync runs now.
    private(set) var running = false
    /// Set by a call of `sync` while one runs.
    @ObservationIgnored private var again = false
    private(set) var state: SyncState

    /// Called with the ids of the NAS of the app that got new values.
    @ObservationIgnored var onImport: (@MainActor (Set<String>) -> Void)?

    @ObservationIgnored private let store: any CollectorStore
    @ObservationIgnored private let accounts: NasAccounts
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let clock: () -> Int64
    @ObservationIgnored private let listNas: ListNas

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "collector")

    init(
        store: any CollectorStore,
        accounts: NasAccounts,
        defaults: UserDefaults,
        clock: @escaping () -> Int64,
        listNas: @escaping ListNas = { try await collectorNas(baseUrl: $0, fingerprint: $1, token: $2) }
    ) {
        self.store = store
        self.accounts = accounts
        self.defaults = defaults
        self.clock = clock
        self.listNas = listNas
        self.state = defaults.data(forKey: Self.stateKey)
            .flatMap { try? JSONDecoder().decode(SyncState.self, from: $0) } ?? SyncState()
        self.paired = Self.load(from: store)
        self.error = state.stopped
    }

    // MARK: - Assignment

    /// Which NAS of the collector go into which NAS of the app: one the
    /// user chose, while that NAS is on this device; else, unless the user
    /// chose "Nicht übernehmen", the one free NAS of the app with the same
    /// certificate.
    var assignments: [String: Assignment] {
        guard let paired else { return [:] }
        let appNas = accounts.accounts
        var result: [String: Assignment] = [:]
        var taken = Set<UUID>()
        for item in nas {
            if let id = paired.assignments[item.id], appNas.contains(where: { $0.id == id }) {
                result[item.id] = Assignment(appNasId: id, automatic: false)
                taken.insert(id)
            }
        }
        for item in nas.sorted(by: { $0.id < $1.id })
        where result[item.id] == nil && !paired.ignored.contains(item.id) {
            let candidates = appNas.filter {
                !taken.contains($0.id) && sameFingerprint($0.fingerprint, item.fingerprint)
            }
            guard candidates.count == 1, let match = candidates.first else { continue }
            result[item.id] = Assignment(appNasId: match.id, automatic: true)
            taken.insert(match.id)
        }
        return result
    }

    // MARK: - Sync

    /// Syncs when paired, not stopped, not running and the last sync is
    /// at least `interval` seconds ago (or in the future).
    @discardableResult
    func syncIfDue(interval: Int64) async -> Set<String> {
        if paired == nil {
            // Paired on another device meanwhile (iCloud Keychain).
            paired = Self.load(from: store)
        }
        guard paired != nil, state.stopped == nil, !running else { return [] }
        let now = clock()
        if let last = state.lastSyncAt, last <= now, now - last < interval {
            return []
        }
        return await sync()
    }

    /// Lists the NAS of the collector and imports every assigned NAS that
    /// the core knows, one after the other. A refused key or a changed
    /// certificate stops the automatic sync; an unreachable collector is
    /// tried again after the next interval. Returns the ids of the NAS of
    /// the app that got new values, and hands them to `onImport`.
    /// A call while a sync runs makes that one run again afterwards, so a
    /// new choice of the user is not lost.
    @discardableResult
    func sync() async -> Set<String> {
        guard paired != nil else { return [] }
        guard !running else {
            again = true
            return []
        }
        running = true
        defer { running = false }
        var imported = Set<String>()
        repeat {
            again = false
            imported.formUnion(await syncOnce())
        } while again && paired != nil
        return imported
    }

    private func syncOnce() async -> Set<String> {
        guard let collector = paired else { return [] }
        state.lastSyncAt = clock()
        saveState()

        let list: [CollectorNas]
        do {
            list = try await listNas(collector.baseUrl, collector.fingerprint, collector.token)
        } catch {
            guard Self.sameTarget(paired, collector) else { return [] }
            let text = collectorErrorText(error)
            self.error = text
            switch error as? FfiError {
            case .LoginFailed?, .CertificateChanged?:
                state.stopped = text
                saveState()
            default:
                break
            }
            Self.log.error("Collector not synced: \(String(describing: error), privacy: .public)")
            return []
        }
        guard Self.sameTarget(paired, collector) else { return [] }
        nas = list
        error = nil
        state.stopped = nil
        saveState()

        var imported = Set<String>()
        var errors: [String: String] = [:]
        for (collectorNasId, assignment) in assignments.sorted(by: { $0.key < $1.key })
        where accounts.isRegistered(assignment.appNasId) {
            let key = assignment.appNasId.uuidString
            let since = state.progress[key].flatMap {
                $0.collectorNasId == collectorNasId ? $0.until : nil
            }
            do {
                let result = try await accounts.core.importFromCollector(
                    baseUrl: collector.baseUrl,
                    fingerprint: collector.fingerprint,
                    token: collector.token,
                    collectorNasId: collectorNasId,
                    nasId: key,
                    since: since
                )
                guard Self.sameTarget(paired, collector) else { return imported }
                state.progress[key] = Progress(collectorNasId: collectorNasId, until: result.until)
                saveState()
                if result.samples > 0 || result.hourly > 0 || result.daily > 0 || result.backupRuns > 0 {
                    imported.insert(key)
                }
            } catch {
                guard Self.sameTarget(paired, collector) else { return imported }
                errors[collectorNasId] = collectorImportErrorText(error)
            }
        }
        nasErrors = errors
        state.lastSuccessAt = clock()
        saveState()
        if !imported.isEmpty {
            onImport?(imported)
        }
        return imported
    }

    // MARK: - Changes of the user

    /// Pairs with the collector: lists its NAS as a test, which throws and
    /// stores nothing on failure; else stores it. Pairing again with the
    /// same address keeps the choices and the progress. Does not sync: the
    /// first import may take long, the caller starts it.
    func pair(baseUrl: String, fingerprint: String, token: String, syncsViaICloud: Bool) async throws {
        _ = try await listNas(baseUrl, fingerprint, token)
        var collector = StoredCollector(
            baseUrl: baseUrl, fingerprint: fingerprint, token: token, syncsViaICloud: syncsViaICloud
        )
        let sameAddress = paired?.baseUrl == baseUrl
        if sameAddress, let paired {
            collector.assignments = paired.assignments
            collector.ignored = paired.ignored
        }
        try store.save(collector)
        paired = collector
        if !sameAddress {
            state.progress = [:]
            nas = []
        }
        state.stopped = nil
        error = nil
        nasErrors = [:]
        saveState()
    }

    /// Takes a NAS of the collector over into `appNasId`, or not at all
    /// with `nil`. Takes effect with the next sync.
    func assign(_ collectorNasId: String, to appNasId: UUID?) throws {
        guard var collector = paired else { return }
        if let appNasId {
            collector.assignments = collector.assignments.filter { $0.value != appNasId }
            collector.assignments[collectorNasId] = appNasId
            collector.ignored.remove(collectorNasId)
        } else {
            collector.assignments[collectorNasId] = nil
            collector.ignored.insert(collectorNasId)
        }
        try store.save(collector)
        paired = collector
    }

    /// Removes the pairing and the progress of this device; the history
    /// taken over stays.
    func remove() throws {
        try store.delete()
        paired = nil
        nas = []
        error = nil
        nasErrors = [:]
        state = SyncState()
        defaults.removeObject(forKey: Self.stateKey)
    }

    /// Moves the keychain entry to or from iCloud Keychain; on failure the
    /// old entry is put back.
    func setSyncsViaICloud(_ syncs: Bool) throws {
        guard let old = paired, old.syncsViaICloud != syncs else { return }
        var collector = old
        collector.syncsViaICloud = syncs
        do {
            try store.save(collector)
        } catch {
            try? store.save(old)
            throw error
        }
        paired = collector
    }

    /// A NAS of the app is gone: its choice and progress go too.
    func forget(appNasId: UUID) {
        state.progress[appNasId.uuidString] = nil
        saveState()
        guard var collector = paired, collector.assignments.values.contains(appNasId) else { return }
        collector.assignments = collector.assignments.filter { $0.value != appNasId }
        do {
            try store.save(collector)
            paired = collector
        } catch {
            Self.log.error("Collector choice not removed: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: - Internals

    /// Whether `current` is still the collector a sync started with:
    /// another choice of the user does not count, another pairing does.
    private static func sameTarget(_ current: StoredCollector?, _ start: StoredCollector) -> Bool {
        guard let current else { return false }
        return current.baseUrl == start.baseUrl
            && current.fingerprint == start.fingerprint
            && current.token == start.token
    }

    private func saveState() {
        guard let data = try? JSONEncoder().encode(state) else { return }
        defaults.set(data, forKey: Self.stateKey)
    }

    private static func load(from store: any CollectorStore) -> StoredCollector? {
        do {
            return try store.load()
        } catch {
            log.error("Collector not read: \(String(describing: error), privacy: .public)")
            return nil
        }
    }
}
