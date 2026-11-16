import Foundation
import Observation
import os

/// Transfer of state, history and events from the Mac to the iPhone over
/// the private iCloud database (#37). The Mac writes after every new
/// snapshot, the iPhone reads what changed since its last fetch. One
/// transfer runs at a time; an error is shown and the next new snapshot
/// (Mac) or fetch (iPhone) tries again. Credentials never go to iCloud.
@MainActor
@Observable
final class CloudTransfer {
    /// What this device does.
    enum Role: Sendable {
        /// Writes (Mac).
        case upload
        /// Reads (iPhone).
        case download

        static var current: Role {
            #if os(macOS)
            .upload
            #else
            .download
            #endif
        }
    }

    /// What the settings show.
    enum Status: Equatable {
        /// Nothing transferred yet.
        case none
        /// Last upload (Mac) or the newest state from the Mac (iPhone).
        case transferred(Date)
        /// iPhone: no Mac wrote anything yet.
        case noData
        case failed(String)
    }

    let role: Role
    private(set) var status: Status

    static let uploadsKey = "cloudUploads"
    static let tokenKey = "cloudChangeToken"
    static let lastTransferKey = "cloudLastTransfer"
    static let fetchedIdsKey = "cloudFetchedIds"
    static let notSignedIn = String(localized: "Nicht bei iCloud angemeldet.")

    @ObservationIgnored private let database: any CloudDatabase
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let clock: () -> Int64
    /// The transfer running now; the next one waits for it.
    @ObservationIgnored private var queue: Task<Void, Never>?
    @ObservationIgnored private var zoneReady = false
    @ObservationIgnored private var subscribed = false
    /// Set by `deleteAll`: uploads still waiting write nothing, until
    /// `resume`.
    @ObservationIgnored private var paused = false

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "cloud")

    init(
        role: Role = .current,
        database: any CloudDatabase,
        defaults: UserDefaults,
        clock: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.role = role
        self.database = database
        self.defaults = defaults
        self.clock = clock
        let last = defaults.object(forKey: Self.lastTransferKey) as? Int64
        self.status = last.map { .transferred(Date(timeIntervalSince1970: TimeInterval($0))) } ?? .none
    }

    /// Start of the last day uploaded per NAS id; it goes again with the
    /// next upload, because it may have grown since.
    var uploads: [String: Int64] {
        (defaults.dictionary(forKey: Self.uploadsKey) as? [String: Int64]) ?? [:]
    }

    // MARK: - Mac

    /// Writes the state of a NAS with its last 24 hours and every day from
    /// the last one uploaded on (the first time: every day of the archive).
    /// The mark moves only after the records are saved.
    func upload(nasId: String, observedAt: Int64, core: any NasCoreProtocol) async {
        await enqueue { await self.runUpload(nasId: nasId, observedAt: observedAt, core: core) }
    }

    /// Deletes everything this Mac wrote, with the zone, after the upload
    /// running now; uploads that come later write nothing until `resume`.
    /// Throws when iCloud refuses; the marks stay then, and uploads go on.
    func deleteAll() async throws {
        paused = true
        let result: Result<Void, any Error> = await enqueue {
            do {
                try await self.database.deleteZone()
                return .success(())
            } catch {
                return .failure(error)
            }
        }
        if case .failure(let error) = result {
            paused = false
            throw error
        }
        zoneReady = false
        defaults.removeObject(forKey: Self.uploadsKey)
        defaults.removeObject(forKey: Self.lastTransferKey)
        status = .none
    }

    /// Uploads write again, after the switch went on.
    func resume() {
        paused = false
    }

    private func runUpload(nasId: String, observedAt: Int64, core: any NasCoreProtocol) async {
        guard !paused else { return }
        do {
            guard let state = try core.savedState(nasId: nasId) else { return }
            guard try await database.accountStatus() == .available else {
                fail(Self.notSignedIn)
                return
            }
            if !zoneReady {
                try await database.ensureZone()
                zoneReady = true
            }
            let now = clock()
            let mark = uploads[nasId]
            let days = try core.historyDays(nasId: nasId).filter { day in mark.map { day >= $0 } ?? true }
            var records: [CloudRecord] = []
            if let record = CloudRecord.state(
                nasId: nasId,
                observedAt: observedAt,
                payload: state,
                recent: try core.exportRecent(nasId: nasId),
                writtenAt: now
            ) {
                records.append(record)
            }
            for day in days {
                guard let json = try core.exportDay(nasId: nasId, day: day),
                      let record = CloudRecord.day(nasId: nasId, day: day, payload: json, writtenAt: now)
                else { continue }
                records.append(record)
            }
            try await database.save(records)
            if let last = days.last {
                var marks = uploads
                marks[nasId] = last
                defaults.set(marks, forKey: Self.uploadsKey)
            }
            transferred(at: now)
        } catch CloudDatabaseError.zoneNotFound {
            // Deleted elsewhere: everything goes again with the next upload.
            zoneReady = false
            defaults.removeObject(forKey: Self.uploadsKey)
            fail(String(localized: "Die Daten in iCloud fehlen, sie werden neu übertragen."))
        } catch {
            fail(connectionErrorText(error))
        }
    }

    // MARK: - iPhone

    /// Fetches what changed since the last fetch and hands the records of
    /// the NAS in `knownIds` to `apply`; others are skipped, as are records
    /// of a newer version. A NAS new in `knownIds` fetches everything again,
    /// so its records skipped before come now. When `apply` returns `false`
    /// (a record was not taken over), the token stays, so they come again.
    /// Returns how many records went to `apply`.
    @discardableResult
    func fetch(knownIds: Set<String>, apply: @escaping @MainActor ([CloudRecord]) -> Bool) async -> Int {
        await enqueue { await self.runFetch(knownIds: knownIds, apply: apply) }
    }

    /// Fetches everything again with the next fetch, e.g. when the switch
    /// goes on.
    func resetToken() {
        defaults.removeObject(forKey: Self.tokenKey)
    }

    private func runFetch(knownIds: Set<String>, apply: @MainActor ([CloudRecord]) -> Bool) async -> Int {
        let fetched = Set(defaults.stringArray(forKey: Self.fetchedIdsKey) ?? [])
        if !knownIds.isSubset(of: fetched) {
            resetToken()
        }
        do {
            guard try await database.accountStatus() == .available else {
                fail(Self.notSignedIn)
                return 0
            }
            let changes: CloudChanges
            do {
                changes = try await database.changes(since: defaults.data(forKey: Self.tokenKey))
            } catch CloudDatabaseError.tokenExpired {
                resetToken()
                // The zone may be new, without the subscription.
                subscribed = false
                changes = try await database.changes(since: nil)
            }
            let usable = changes.records.filter { record in
                record.version <= CloudRecord.version && knownIds.contains(record.nasId)
            }
            if apply(usable) {
                defaults.set(changes.token, forKey: Self.tokenKey)
                defaults.set(knownIds.sorted(), forKey: Self.fetchedIdsKey)
            }
            if let newest = usable.filter({ $0.kind == .state }).map(\.writtenAt).max() {
                transferred(at: newest)
            } else {
                status = lastTransfer.map(Status.transferred) ?? .none
            }
            await subscribeOnce()
            return usable.count
        } catch CloudDatabaseError.zoneNotFound {
            // A subscription needs the zone; it follows with the first fetch
            // that finds one.
            resetToken()
            subscribed = false
            status = .noData
        } catch {
            fail(connectionErrorText(error))
        }
        return 0
    }

    /// A silent push after every change of the zone wakes the app.
    private func subscribeOnce() async {
        guard !subscribed else { return }
        do {
            try await database.subscribe()
            subscribed = true
        } catch {
            Self.log.error("No subscription: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: - Internals

    private var lastTransfer: Date? {
        (defaults.object(forKey: Self.lastTransferKey) as? Int64).map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }

    private func transferred(at time: Int64) {
        defaults.set(time, forKey: Self.lastTransferKey)
        status = .transferred(Date(timeIntervalSince1970: TimeInterval(time)))
    }

    private func fail(_ message: String) {
        Self.log.error("Transfer failed: \(message, privacy: .public)")
        status = .failed(message)
    }

    private func enqueue<T: Sendable>(_ work: @escaping @MainActor () async -> T) async -> T {
        let previous = queue
        let task = Task {
            await previous?.value
            return await work()
        }
        queue = Task { _ = await task.value }
        return await task.value
    }
}
