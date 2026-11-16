import Foundation
import Observation
import os
import WidgetKit

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
                requestNotifications()
                Task { await refreshAll() }
            }
        }
    }

    /// Thresholds of the settings (#21); the core computes the findings of
    /// every entry with them (#85), also in demo mode. Saved under
    /// `thresholds` in the defaults.
    var thresholds: AlertThresholds {
        didSet {
            guard thresholds != oldValue else { return }
            thresholds.save(to: defaults)
            accounts.core.setThresholds(thresholds: thresholds.core)
            rebuild()
        }
    }

    /// Minutes between two queries of the same NAS, one of
    /// `intervalChoices`; never less than 5 (`minInterval`). Saved under
    /// `queryInterval`; a change queries nothing by itself.
    var queryIntervalMinutes: Int {
        didSet {
            guard queryIntervalMinutes != oldValue else { return }
            defaults.set(queryIntervalMinutes, forKey: Self.queryIntervalKey)
            // The widget snapshot holds the age from which a state is old.
            onUpdate?(entries)
        }
    }

    /// Which kinds of problems notify and when not (#39). Saved under
    /// `notifications` in the defaults.
    var notificationSettings: NotificationSettings {
        didSet {
            guard notificationSettings != oldValue else { return }
            notificationSettings.save(to: defaults)
            notifier?.apply(notificationSettings, previous: oldValue)
        }
    }

    /// Called after every new state with the sorted entries and after a
    /// change of the interval; `live()` writes the widget snapshot here and
    /// reloads the widgets (#23).
    @ObservationIgnored var onUpdate: (@MainActor ([NasEntry]) -> Void)?

    static let demoModeKey = "demoMode"
    static let queryLogKey = "queryLog"
    static let queryIntervalKey = "queryInterval"
    /// Choices of the settings, in minutes.
    static let intervalChoices = [5, 10, 15, 30]
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
    @ObservationIgnored private let demoSource: (Int64, Thresholds) -> [Snapshot]
    @ObservationIgnored private let pollInterval: Duration?
    @ObservationIgnored private let clock: () -> Int64
    @ObservationIgnored private let lastStates: LastStates
    /// Notifications of the open events (#39); `nil` notifies nothing.
    @ObservationIgnored private let notifier: EventNotifier?
    /// What `lastStates` holds: NAS id → saved state of the core.
    @ObservationIgnored private var savedStates: [String: String]

    @ObservationIgnored private var queryErrors: [String: String] = [:]
    @ObservationIgnored private var log: QueryLog
    /// Snapshot of the test query of the add dialog, until the core has one;
    /// the real core takes it over in `addNas` at once, so its findings
    /// follow the thresholds.
    @ObservationIgnored private var testSnapshots: [String: Snapshot] = [:]
    @ObservationIgnored private var started = false
    /// Lives as long as the store; ends once the store is gone.
    @ObservationIgnored private var pollTask: Task<Void, Never>?
    /// The round of `refreshAll` running now; a second call waits for it.
    @ObservationIgnored private var round: Task<Void, Never>?
    /// The request for permission of `requestNotifications`; the tests wait
    /// for it.
    @ObservationIgnored private(set) var permissionRequest: Task<Void, Never>?

    /// - Parameter pollInterval: Time between two rounds of queries; `nil`
    ///   queries only on `start`, `add`, `updateCredentials` and explicit
    ///   `refreshAll` calls.
    /// - Parameter lastStates: Where the last snapshot of every NAS is
    ///   kept across launches; by default nowhere.
    /// - Parameter notifier: Notifies the open events after every query;
    ///   by default none.
    init(
        accounts: NasAccounts,
        defaults: UserDefaults,
        demoSource: @escaping (Int64, Thresholds) -> [Snapshot],
        pollInterval: Duration?,
        clock: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        lastStates: LastStates = LastStates(fileURL: nil),
        notifier: EventNotifier? = nil
    ) {
        self.clock = clock
        self.lastStates = lastStates
        self.notifier = notifier
        self.accounts = accounts
        self.defaults = defaults
        self.demoSource = demoSource
        self.pollInterval = pollInterval
        self.log = defaults.data(forKey: Self.queryLogKey)
            .flatMap { try? JSONDecoder().decode(QueryLog.self, from: $0) } ?? QueryLog()
        self.demoMode = defaults.bool(forKey: Self.demoModeKey)
        let thresholds = AlertThresholds.load(from: defaults)
        self.thresholds = thresholds
        self.queryIntervalMinutes = Self.storedInterval(in: defaults)
        self.notificationSettings = NotificationSettings.load(from: defaults)
        // Before the first state, so restored ones get the same findings.
        accounts.core.setThresholds(thresholds: thresholds.core)
        // The last state of an earlier launch shows until the first answer;
        // one the core refuses (other format after an update) is dropped,
        // as is one of a NAS no longer stored (e.g. removed on another
        // device). An unreadable keychain drops nothing.
        var saved = lastStates.load()
        let before = saved
        if accounts.loadError == nil {
            let known = Set(accounts.accounts.map(\.id.uuidString))
            saved = saved.filter { known.contains($0.key) }
        }
        for account in accounts.accounts where accounts.isRegistered(account.id) {
            let id = account.id.uuidString
            guard let json = saved[id] else { continue }
            do {
                try accounts.core.restoreState(nasId: id, json: json)
            } catch {
                saved[id] = nil
            }
        }
        self.savedStates = saved
        if saved != before {
            lastStates.save(saved)
        }
        // The list shows the stored NAS at once, before the first query.
        rebuild()
    }

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "store")

    /// `history.sqlite` in the container of the App Group, the archive of
    /// history and events of the core; `nil` without the container.
    static var historyURL: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
            .appendingPathComponent("history.sqlite")
    }

    /// The store of the app: the NAS from the keychain, the demo data of the
    /// core, demo switch and settings in the defaults of the App Group and
    /// a poll round every 5 minutes (each NAS at its `queryIntervalMinutes`).
    /// Every new state goes into the widget snapshot of the App Group (#23).
    /// The archive of the core is `history.sqlite`, so the events and with
    /// them the notifications (#39) last across launches.
    static func live() -> NasStore {
        let isPreview = ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1"
        let defaults = UserDefaults(suiteName: AppGroup.id) ?? .standard
        let store = NasStore(
            accounts: NasAccounts(core: isPreview ? NasCore() : liveCore()),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            pollInterval: .seconds(300),
            lastStates: LastStates(fileURL: isPreview ? nil : LastStates.liveURL),
            notifier: isPreview ? nil : EventNotifier(poster: SystemNotificationPoster(), defaults: defaults)
        )
        // Previews never touch the files of the real app.
        guard !isPreview else {
            return store
        }
        // Without a container the writer does nothing.
        let writer = WidgetSnapshotWriter(fileURL: WidgetSnapshot.fileURL) {
            WidgetCenter.shared.reloadAllTimelines()
        }
        store.onUpdate = { [weak store] entries in
            guard let store else { return }
            writer.update(WidgetSnapshot(
                entries: entries,
                demo: store.demoMode,
                queryIntervalMinutes: store.queryIntervalMinutes,
                thresholds: store.thresholds
            ))
        }
        store.onUpdate?(store.entries)
        return store
    }

    /// Core with the archive in `historyURL`; without the container or when
    /// the file does not open, one with the archive in memory.
    private static func liveCore() -> NasCore {
        guard let url = historyURL else { return NasCore() }
        do {
            return try NasCore.withHistory(path: url.path)
        } catch {
            log.error("History not opened: \(String(describing: error), privacy: .public)")
            return NasCore()
        }
    }

    /// Whether the user switched notifications off for the app; `false`
    /// without a notifier.
    func notificationsDenied() async -> Bool {
        await notifier?.isDenied() ?? false
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
        requestNotifications()
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
    /// queried within the interval of the settings (at least 5 minutes),
    /// also in an earlier launch or by the test query of the add dialog. Does nothing in demo mode.
    /// While a round runs, a further call waits for it instead of starting
    /// one of its own (start, return to the app, background task).
    func refreshAll() async {
        guard !demoMode else { return }
        if let round {
            await Self.wait(for: round)
            return
        }
        let ids = queryable()
        // Runs on the main actor after this call, so `round` is set first.
        round = Task {
            await self.query(ids)
            self.round = nil
        }
        if let round {
            await Self.wait(for: round)
        }
    }

    /// Waits for `task`, but returns at once when the calling task is
    /// cancelled (e.g. the background task when iOS ends it); `task`
    /// itself runs on.
    private static func wait(for task: Task<Void, Never>) async {
        let finished = AsyncStream<Void> { continuation in
            Task {
                await task.value
                continuation.finish()
            }
        }
        for await _ in finished {}
    }

    // MARK: - Changing the list

    /// Stores a NAS after the test query of the add dialog. That query
    /// counts as its first: its snapshot shows until the core has its own,
    /// and the next query follows with the poll round after the interval of
    /// the settings.
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
        requestNotifications()
    }

    /// Removes a NAS from the keychain, the core and the list.
    func remove(id: UUID) async throws {
        try await accounts.remove(id: id)
        queryErrors[id.uuidString] = nil
        testSnapshots[id.uuidString] = nil
        log.blocked[id.uuidString] = nil
        log.lastAttempts[id.uuidString] = nil
        saveLog()
        if savedStates.removeValue(forKey: id.uuidString) != nil {
            lastStates.save(savedStates)
        }
        notifier?.forget(nasId: id.uuidString)
        rebuild()
    }

    /// New user name and password for a stored NAS, e.g. after a refused
    /// login; lifts a block and queries again. When the core refuses them,
    /// the old credentials stay stored.
    func updateCredentials(id: UUID, user: String, password: String) async throws {
        try await unblock(id) {
            try await accounts.updateCredentials(id: id, user: user, password: password)
        }
    }

    /// New name for a stored NAS; empty keeps the old one (throws).
    func rename(id: UUID, to name: String) throws {
        try accounts.rename(id: id, name: name)
        rebuild()
    }

    /// Switches the iCloud Keychain setting of a NAS (#98). No query: the
    /// NAS and its state stay as they are.
    func setSyncsViaICloud(id: UUID, to syncs: Bool) throws {
        try accounts.setSyncsViaICloud(id: id, syncs: syncs)
    }

    /// "Erneut versuchen": starts a blocked NAS again with its stored
    /// credentials and queries it once. A step of the user, so no automatic
    /// retry; when it fails again, the NAS stays blocked.
    func resume(id: UUID) async throws {
        try await unblock(id) {
            try await accounts.resume(id: id)
        }
    }

    /// Trusts the new certificate of a NAS whose certificate changed (#76),
    /// lifts the block and queries again. When the core refuses, the old
    /// fingerprint stays stored.
    func confirmFingerprint(id: UUID, fingerprint: String) async throws {
        try await unblock(id) {
            try await accounts.confirmFingerprint(id: id, fingerprint: fingerprint)
        }
    }

    // MARK: - Internals

    /// Runs `change` with the core; on success the block and the last
    /// error go and the NAS is queried once (not in demo mode). On failure
    /// the entry shows the reason and the error is thrown.
    private func unblock(_ id: UUID, _ change: () async throws -> Void) async throws {
        do {
            try await change()
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

    /// Asks for notifications once there is a NAS to query (not in demo
    /// mode); the system asks the user only the first time. Once given,
    /// the open events in the archive notify at once, without a query: the
    /// first query of a launch often ends while the user still decides.
    private func requestNotifications() {
        guard let notifier, !demoMode, !accounts.accounts.isEmpty else { return }
        permissionRequest = Task {
            await notifier.requestAuthorization()
            let before = notifier.authorized
            await notifier.refreshAuthorization()
            guard notifier.authorized, !before else { return }
            for account in accounts.accounts where accounts.isRegistered(account.id) {
                notifyEvents(of: account.id.uuidString)
            }
        }
    }

    /// The stored interval; a missing or unknown value is 5 minutes.
    private static func storedInterval(in defaults: UserDefaults) -> Int {
        let stored = defaults.integer(forKey: queryIntervalKey)
        return intervalChoices.contains(stored) ? stored : intervalChoices[0]
    }

    /// Seconds between two queries of the same NAS.
    private var queryIntervalSeconds: Int64 {
        max(Self.minInterval, Int64(queryIntervalMinutes) * 60)
    }

    /// Registered NAS that are not blocked and were not queried within the
    /// interval of the settings (at least 5 minutes).
    private func queryable() -> [UUID] {
        let now = clock()
        return accounts.accounts.map(\.id).filter { id in
            let key = id.uuidString
            guard accounts.isRegistered(id), log.blocked[key] == nil else { return false }
            guard let last = log.lastAttempts[key] else { return true }
            return now < last || now - last >= queryIntervalSeconds
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
        // Before the answers, so `record` knows whether it may notify.
        await notifier?.refreshAuthorization()
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
                saveState(of: key)
            }
        }
        saveLog()
        rebuild()
        if state != nil {
            notifyEvents(of: key)
        }
    }

    /// Hands the open events of a NAS to the notifier, after `refresh`
    /// wrote them to the archive. Open to the end, because `startedAt` is
    /// the clock of the NAS. A failing archive notifies nothing. The demo
    /// mode queries nothing and so notifies nothing.
    /// The callback of the planner (#67) must call this too.
    private func notifyEvents(of key: String) {
        guard let notifier, !demoMode else { return }
        guard let events = try? accounts.core.events(nasId: key, from: clock(), to: Int64.max) else { return }
        let name = entries.first { $0.id == key }?.displayName ?? key
        notifier.update(
            nasId: key,
            name: name,
            open: events.filter { $0.endedAt == nil },
            settings: notificationSettings
        )
    }

    /// Keeps the last snapshot of the core for the next launch; a failure
    /// leaves the saved one.
    private func saveState(of key: String) {
        guard let json = try? accounts.core.savedState(nasId: key),
              savedStates[key] != json
        else { return }
        savedStates[key] = json
        lastStates.save(savedStates)
    }

    /// Builds `entries` from the accounts and the states of the core (or
    /// from the demo data), sorts them and reports them.
    private func rebuild() {
        if demoMode {
            let now = Int64(Date().timeIntervalSince1970)
            entries = demoSource(now, thresholds.core).map { snapshot in
                NasEntry(
                    id: snapshot.nas.id,
                    name: snapshot.nas.name,
                    baseUrl: snapshot.nas.baseUrl,
                    provider: snapshot.nas.provider,
                    snapshot: snapshot,
                    thresholds: thresholds
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
                    queryError: queryErrors[id],
                    thresholds: thresholds
                )
            }
        }
        entries.sort(by: NasEntry.ordered)
        onUpdate?(entries)
    }
}
