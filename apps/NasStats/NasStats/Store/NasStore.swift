import Foundation
import Observation
import os
import WidgetKit

/// The state of every NAS for all screens, the menu bar and the widgets:
/// holds the last state per NAS of `NasAccounts`, worst first, with the
/// forecast of every volume and the recent fill level of the fullest one,
/// and hands out history and events of the core (#67).
///
/// The planner of the core queries the NAS when they are due and reports
/// every finished query through `StoreListener`; the store itself queries
/// only on `refreshAll` (return to the app, background task, "Aktualisieren")
/// and after a change of the access.
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
            if demoMode {
                accounts.core.stopPlanner()
                enterDemo()
            } else {
                demoCoreInstance = nil
                demoDerived = [:]
            }
            rebuild()
            if !demoMode, started {
                requestNotifications()
                accounts.core.startPlanner(listener: listener)
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
            demoCoreInstance?.setThresholds(thresholds: thresholds.core)
            rebuild()
        }
    }

    /// Minutes between two queries of the same NAS, one of
    /// `intervalChoices`; never less than 5 (`minInterval`). Saved under
    /// `queryInterval`; a change queries nothing by itself, the planner of
    /// the core takes it over.
    var queryIntervalMinutes: Int {
        didSet {
            guard queryIntervalMinutes != oldValue else { return }
            defaults.set(queryIntervalMinutes, forKey: Self.queryIntervalKey)
            accounts.core.setInterval(secs: queryIntervalSeconds)
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

    /// Whether the NAS of this device are kept in iCloud Keychain, so they
    /// show on every device with the same Apple Account (#113). One setting
    /// for all NAS, on by default; it belongs to this device and is saved
    /// under `sharesAccessViaICloud` in the defaults. Change it with
    /// `setSharesAccessViaICloud`.
    private(set) var sharesAccessViaICloud: Bool

    /// Whether state, history and events go over the private iCloud from
    /// the Mac to the iPhone (#37): the Mac writes them after every new
    /// snapshot, the iPhone takes them over for the NAS of its own list.
    /// Off by default, one setting per device, saved under
    /// `transfersViaICloud`. Change it with `setTransfersViaICloud`.
    private(set) var transfersViaICloud: Bool

    /// The transfer over iCloud; `nil` transfers nothing (tests, previews).
    @ObservationIgnored let cloud: CloudTransfer?
    /// The last upload or fetch started by the store; the tests wait for it.
    @ObservationIgnored private(set) var cloudWork: Task<Void, Never>?

    static let transfersViaICloudKey = "transfersViaICloud"
    static let sharesAccessViaICloudKey = "sharesAccessViaICloud"
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
    /// Core with history and events of the demo data set, for a time.
    @ObservationIgnored private let demoCore: (Int64) -> any NasCoreProtocol
    /// The demo core while the demo mode is on; never has a planner.
    @ObservationIgnored private var demoCoreInstance: (any NasCoreProtocol)?
    @ObservationIgnored private let clock: () -> Int64
    /// Receives the states of the planner of the core.
    @ObservationIgnored private let listener: StoreListener
    /// Forecasts and recent fill level per NAS id, from the history of the
    /// core; filled with every new snapshot, read by `rebuild`.
    @ObservationIgnored private var derived: [String: Derived] = [:]
    /// The same for the demo NAS, from the demo core.
    @ObservationIgnored private var demoDerived: [String: Derived] = [:]
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
    /// The round of `refreshAll` running now; a second call waits for it.
    @ObservationIgnored private var round: Task<Void, Never>?
    /// The request for permission of `requestNotifications`; the tests wait
    /// for it.
    @ObservationIgnored private(set) var permissionRequest: Task<Void, Never>?

    private static let logger = Logger(subsystem: "net.staude.nasstats", category: "store")

    /// - Parameter demoCore: Core with the history and events of the demo
    ///   data set at a time (Unix seconds UTC), made when the demo mode
    ///   goes on.
    /// - Parameter lastStates: Where the last snapshot of every NAS is
    ///   kept across launches; by default nowhere.
    /// - Parameter notifier: Notifies the open events after every query;
    ///   by default none.
    /// - Parameter cloud: Transfer over iCloud (#37); by default none.
    init(
        accounts: NasAccounts,
        defaults: UserDefaults,
        demoSource: @escaping (Int64, Thresholds) -> [Snapshot],
        demoCore: @escaping (Int64) -> any NasCoreProtocol = { NasCore.demo(now: $0) },
        clock: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        lastStates: LastStates = LastStates(fileURL: nil),
        notifier: EventNotifier? = nil,
        cloud: CloudTransfer? = nil
    ) {
        self.clock = clock
        self.cloud = cloud
        self.transfersViaICloud = defaults.bool(forKey: Self.transfersViaICloudKey)
        self.lastStates = lastStates
        self.notifier = notifier
        self.accounts = accounts
        self.defaults = defaults
        self.demoSource = demoSource
        self.demoCore = demoCore
        let (stream, continuation) = AsyncStream.makeStream(of: NasState.self)
        self.listener = StoreListener(continuation)
        self.log = defaults.data(forKey: Self.queryLogKey)
            .flatMap { try? JSONDecoder().decode(QueryLog.self, from: $0) } ?? QueryLog()
        self.demoMode = defaults.bool(forKey: Self.demoModeKey)
        let thresholds = AlertThresholds.load(from: defaults)
        self.thresholds = thresholds
        self.queryIntervalMinutes = Self.storedInterval(in: defaults)
        self.notificationSettings = NotificationSettings.load(from: defaults)
        let storedShares = defaults.object(forKey: Self.sharesAccessViaICloudKey) as? Bool
        self.sharesAccessViaICloud = storedShares ?? true
        // First launch with the global setting: the NAS kept only on this
        // device so far follow it once, so "on" holds. With an unreadable
        // keychain nothing is saved, so it runs again at the next launch.
        if storedShares == nil, accounts.loadError == nil {
            defaults.set(true, forKey: Self.sharesAccessViaICloudKey)
            do {
                let failed = try accounts.setSyncsViaICloud(all: true)
                if failed > 0 {
                    Self.logger.error("First move to iCloud Keychain: \(failed, privacy: .public) NAS not moved")
                }
            } catch {
                Self.logger.error("First move to iCloud Keychain failed: \(String(describing: error), privacy: .public)")
            }
        }
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
        // Forecasts of the restored states, from the archive of earlier
        // launches.
        let now = clock()
        for state in accounts.core.states() {
            guard let snapshot = state.snapshot,
                  let id = UUID(uuidString: state.nasId), accounts.isRegistered(id)
            else { continue }
            derived[state.nasId] = Self.derive(state.nasId, snapshot: snapshot, core: accounts.core, now: now)
        }
        if demoMode {
            enterDemo()
        }
        // The list shows the stored NAS at once, before the first query.
        rebuild()
        // Every state of the planner, on the main actor; ends with the store.
        Task { [weak self] in
            for await state in stream {
                guard let self else { return }
                // So `record` knows whether it may notify (#39).
                await self.notifier?.refreshAuthorization()
                self.receive(state)
            }
        }
    }

    deinit {
        listener.finish()
    }

    /// `history.sqlite` in the container of the App Group, the archive of
    /// history and events of the core; `nil` without the container.
    static var historyURL: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: AppGroup.id)?
            .appendingPathComponent("history.sqlite")
    }

    /// The store of the app: the NAS from the keychain with the history
    /// archive in the App Group, the demo data of the core, demo switch and
    /// settings in the defaults of the App Group; the planner of the core
    /// queries each NAS at its `queryIntervalMinutes`.
    /// Every new state goes into the widget snapshot of the App Group (#23).
    /// The archive of the core is `history.sqlite`, so the events and with
    /// them the notifications (#39) last across launches.
    static func live() -> NasStore {
        let isPreview = ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1"
        let defaults = UserDefaults(suiteName: isPreview ? "preview-live" : AppGroup.id) ?? .standard
        // Previews get an empty keychain and defaults of their own, so
        // neither the first move to iCloud Keychain nor the switch of the
        // settings (#113) touches the real entries or marks the move done,
        // and no real NAS data shows. Their core keeps its archive in memory.
        let store = NasStore(
            accounts: isPreview
                ? NasAccounts(keychain: NoAccountStore(), core: NasCore())
                : NasAccounts(core: liveCore()),
            defaults: defaults,
            demoSource: { demoSnapshots(now: $0, thresholds: $1) },
            lastStates: LastStates(fileURL: isPreview ? nil : LastStates.liveURL),
            notifier: isPreview ? nil : EventNotifier(poster: SystemNotificationPoster(), defaults: defaults),
            cloud: isPreview ? nil : CloudTransfer(database: CloudKitDatabase(), defaults: defaults)
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
            logger.error("History not opened: \(String(describing: error), privacy: .public)")
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

    /// Hands the time of the last query and the blocks of earlier launches
    /// to the core, sets the interval and starts its planner (not in demo
    /// mode), which queries every NAS when it is due. Queries nothing
    /// itself. Runs once, later calls do nothing.
    func start() {
        guard !started else { return }
        started = true
        requestNotifications()
        let core = accounts.core
        for account in accounts.accounts where accounts.isRegistered(account.id) {
            let id = account.id.uuidString
            try? core.restorePoll(nasId: id, lastAttemptAt: log.lastAttempts[id], blocked: log.blocked[id] != nil)
        }
        core.setInterval(secs: queryIntervalSeconds)
        if !demoMode {
            core.startPlanner(listener: listener)
        }
        cloudWork = Task { _ = await self.fetchFromCloud() }
    }

    /// History of a metric of a NAS in a range ending now; in demo mode
    /// from the demo data set. Errors of the core are thrown.
    func history(nasId: String, metric: Metric, range: HistoryRange) throws -> [HistorySeries] {
        try historyCore.history(nasId: nasId, metric: metric, range: range)
    }

    /// Forecast "full in X days" of a volume from the history; `nil`
    /// without a rising trend. In demo mode from the demo data set. Errors
    /// of the core are thrown.
    func forecast(nasId: String, volumeId: String) throws -> Forecast? {
        try historyCore.forecast(nasId: nasId, volumeId: volumeId)
    }

    /// Events of one NAS (`nil`: of all) open at some time in `[from, to]`
    /// (Unix seconds UTC), newest first; in demo mode from the demo data
    /// set. Errors of the core are thrown.
    func events(nasId: String?, from: Int64, to: Int64) throws -> [NasEvent] {
        try historyCore.events(nasId: nasId, from: from, to: to)
    }

    /// The core that holds the history of the entries shown.
    private var historyCore: any NasCoreProtocol {
        demoMode ? (demoCoreInstance ?? accounts.core) : accounts.core
    }

    /// Queries every registered NAS in parallel, except those blocked or
    /// queried within the interval of the settings (at least 5 minutes),
    /// also in an earlier launch or by the test query of the add dialog. Does nothing in demo mode.
    /// While a round runs, a further call waits for it instead of starting
    /// one of its own (return to the app, background task, "Aktualisieren").
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
    /// and the planner queries it next after the interval of the settings.
    /// Throws when the keychain or the core refuses it; nothing changes then.
    /// The entry follows the iCloud Keychain setting of the settings.
    func add(_ stored: StoredAccount, snapshot: Snapshot? = nil) async throws {
        var stored = stored
        stored.account.syncsViaICloud = sharesAccessViaICloud
        // The core registers a new NAS as due and wakes its planner at once;
        // held until the test query counts, it does not query again.
        let planning = started && !demoMode
        if planning {
            accounts.core.stopPlanner()
        }
        defer {
            if planning {
                accounts.core.startPlanner(listener: listener)
                catchUp()
            }
        }
        try accounts.add(stored)
        let id = stored.account.id.uuidString
        try? accounts.core.restorePoll(nasId: id, lastAttemptAt: clock(), blocked: false)
        queryErrors[id] = nil
        testSnapshots[id] = snapshot
        log.blocked[id] = nil
        log.lastAttempts[id] = clock()
        saveLog()
        rebuild()
        requestNotifications()
        // Its state may be in iCloud already, skipped before (#37).
        cloudWork = Task { _ = await self.fetchFromCloud() }
    }

    /// Removes a NAS from the keychain, the core and the list.
    func remove(id: UUID) async throws {
        try await accounts.remove(id: id)
        queryErrors[id.uuidString] = nil
        testSnapshots[id.uuidString] = nil
        derived[id.uuidString] = nil
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

    /// Switches the global iCloud Keychain setting (#113) and moves every
    /// stored NAS with it. Throws when the keychain could not be read at
    /// start; the setting stays then. Otherwise the setting is saved, also
    /// when single entries could not be moved, and their number is
    /// returned. No query: the NAS and their state stay as they are.
    @discardableResult
    func setSharesAccessViaICloud(_ shares: Bool) throws -> Int {
        let failed = try accounts.setSyncsViaICloud(all: shares)
        sharesAccessViaICloud = shares
        defaults.set(shares, forKey: Self.sharesAccessViaICloudKey)
        return failed
    }

    /// Switches the transfer over iCloud (#37). On, the Mac writes every NAS
    /// with a state, the iPhone fetches everything there is. Off, the Mac
    /// deletes what it wrote (the settings ask first) and throws when
    /// iCloud refuses, the switch stays on then; the iPhone only stops
    /// taking over and keeps what it took.
    func setTransfersViaICloud(_ on: Bool) async throws {
        guard on != transfersViaICloud, let cloud else { return }
        if !on, cloud.role == .upload {
            try await cloud.deleteAll()
        }
        transfersViaICloud = on
        defaults.set(on, forKey: Self.transfersViaICloudKey)
        guard on else { return }
        switch cloud.role {
        case .upload:
            for state in accounts.core.states() {
                guard let snapshot = state.snapshot else { continue }
                upload(state.nasId, observedAt: snapshot.observedAt)
            }
            await cloudWork?.value
        case .download:
            cloud.resetToken()
            await fetchFromCloud()
        }
    }

    /// iPhone: takes over what the Mac wrote to iCloud since the last fetch
    /// (#37), for the NAS of this device. Does nothing on the Mac, with
    /// the switch off and in demo mode. `true` when something came.
    @discardableResult
    func fetchFromCloud() async -> Bool {
        guard let cloud, cloud.role == .download, transfersViaICloud, !demoMode else { return false }
        let known = Set(accounts.accounts.filter { accounts.isRegistered($0.id) }.map(\.id.uuidString))
        let count = await cloud.fetch(knownIds: known) { [weak self] records in
            self?.applyRemote(records)
        }
        return count > 0
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
            if let snapshot = state.snapshot {
                testSnapshots[key] = nil
                saveState(of: key)
                // A query of `refreshAll` also comes through the listener.
                if derived[key]?.observedAt != snapshot.observedAt {
                    derived[key] = Self.derive(key, snapshot: snapshot, core: accounts.core, now: clock())
                    upload(key, observedAt: snapshot.observedAt)
                }
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
    /// States of the planner reach it through `record` as well.
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

    /// Mac: writes a new state of a NAS to iCloud (#37), not in demo mode
    /// and only with the switch on.
    private func upload(_ key: String, observedAt: Int64) {
        guard let cloud, cloud.role == .upload, transfersViaICloud, !demoMode else { return }
        let core = accounts.core
        cloudWork = Task { await cloud.upload(nasId: key, observedAt: observedAt, core: core) }
    }

    /// iPhone: takes over the records of the Mac (#37), per NAS the days
    /// first, then the last 24 hours and the state; a state newer than the
    /// own one replaces it and clears the error of the own query. Events
    /// of the Mac show in the chronicle but never notify here: the Mac
    /// notifies them.
    private func applyRemote(_ records: [CloudRecord]) {
        guard !records.isEmpty, !demoMode else { return }
        let core = accounts.core
        for (key, ofNas) in Dictionary(grouping: records, by: \.nasId) {
            guard let id = UUID(uuidString: key), accounts.isRegistered(id) else { continue }
            for day in ofNas.filter({ $0.kind == .day }).sorted(by: { $0.time < $1.time }) {
                importHistory(day.payloadText, of: key)
            }
            if let state = ofNas.first(where: { $0.kind == .state }) {
                importHistory(state.recentText, of: key)
                if let json = state.payloadText {
                    do {
                        if try core.applyRemoteState(nasId: key, json: json) {
                            queryErrors[key] = nil
                            testSnapshots[key] = nil
                        }
                    } catch {
                        Self.logger.error("State of the Mac refused: \(String(describing: error), privacy: .public)")
                    }
                }
            }
            saveState(of: key)
            if let snapshot = core.states().first(where: { $0.nasId == key })?.snapshot {
                derived[key] = Self.derive(key, snapshot: snapshot, core: core, now: clock())
            }
            if let events = try? core.events(nasId: key, from: clock(), to: Int64.max) {
                notifier?.adopt(nasId: key, open: events.filter { $0.endedAt == nil })
            }
        }
        rebuild()
    }

    private func importHistory(_ json: String?, of key: String) {
        guard let json else { return }
        do {
            try accounts.core.importHistory(nasId: key, json: json)
        } catch {
            Self.logger.error("History of the Mac refused: \(String(describing: error), privacy: .public)")
        }
    }

    /// Takes in every query the planner finished while it had no listener
    /// (`add` holds it): a state whose last query is newer than the log.
    private func catchUp() {
        for state in accounts.core.states() {
            guard let id = UUID(uuidString: state.nasId),
                  let last = state.lastAttemptAt, last > (log.lastAttempts[state.nasId] ?? .min)
            else { continue }
            record(id, state: state, error: nil)
        }
    }

    /// A state of the planner; only NAS of the store count.
    private func receive(_ state: NasState) {
        guard let id = UUID(uuidString: state.nasId) else { return }
        record(id, state: state, error: nil)
    }

    /// Makes the demo core and the forecasts of the demo NAS, both at the
    /// same time as the demo snapshots; the clock of the demo core runs on
    /// from there.
    private func enterDemo() {
        let now = Int64(Date().timeIntervalSince1970)
        let core = demoCore(now)
        core.setThresholds(thresholds: thresholds.core)
        demoCoreInstance = core
        demoDerived = Dictionary(
            demoSource(now, thresholds.core).map { snapshot in
                (snapshot.nas.id, Self.derive(snapshot.nas.id, snapshot: snapshot, core: core, now: now))
            },
            uniquingKeysWith: { first, _ in first }
        )
    }

    /// What the store takes from the history of a NAS with a new snapshot.
    struct Derived {
        /// Time of the snapshot it was made for.
        var observedAt: Int64?
        var forecasts: [String: FillForecast] = [:]
        var fullestVolumeHistory: [WidgetFillPoint] = []
    }

    /// Forecast of every volume that counts and the daily fill level of the
    /// fullest one over the 30 days before `now`. A volume without a rising
    /// trend or a failing core gives no forecast, never "kein Trend"; a
    /// failing history gives none.
    static func derive(_ nasId: String, snapshot: Snapshot, core: any NasCoreProtocol, now: Int64) -> Derived {
        let volumes = NasEntry.capacityVolumes(of: snapshot)
        var result = Derived(observedAt: snapshot.observedAt)
        for volume in volumes {
            guard let forecast = try? core.forecast(nasId: nasId, volumeId: volume.id) else { continue }
            result.forecasts[volume.id] = .full(
                days: forecast.daysToFull,
                date: Date(timeIntervalSince1970: TimeInterval(forecast.fullAt))
            )
        }
        // Volumes without a fill level are left out.
        let fullest = volumes
            .compactMap { volume in volume.usedPercent.map { (volume, $0) } }
            .max { $0.1 < $1.1 }?.0
        guard let fullest,
              let used = try? core.history(nasId: nasId, metric: .volumeUsed, range: .year),
              let total = try? core.history(nasId: nasId, metric: .volumeTotal, range: .year)
        else { return result }
        let totals = Dictionary(
            (total.first { $0.key == fullest.id }?.points ?? []).map { ($0.at, $0.mean) },
            uniquingKeysWith: { first, _ in first }
        )
        let since = now - 30 * 86_400
        result.fullestVolumeHistory = (used.first { $0.key == fullest.id }?.points ?? [])
            .filter { $0.at >= since }
            .compactMap { point in
                guard let size = totals[point.at] ?? fullest.totalBytes.map({ Double($0) }), size > 0 else {
                    return nil
                }
                return WidgetFillPoint(
                    day: Date(timeIntervalSince1970: TimeInterval(point.at)),
                    percent: point.mean / size * 100
                )
            }
        return result
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
                let derived = demoDerived[snapshot.nas.id]
                return NasEntry(
                    id: snapshot.nas.id,
                    name: snapshot.nas.name,
                    baseUrl: snapshot.nas.baseUrl,
                    provider: snapshot.nas.provider,
                    snapshot: snapshot,
                    thresholds: thresholds,
                    forecasts: derived?.forecasts ?? [:],
                    fullestVolumeHistory: derived?.fullestVolumeHistory ?? []
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
                    thresholds: thresholds,
                    forecasts: derived[id]?.forecasts ?? [:],
                    fullestVolumeHistory: derived[id]?.fullestVolumeHistory ?? []
                )
            }
        }
        entries.sort(by: NasEntry.ordered)
        onUpdate?(entries)
    }
}

/// Hands every state of the planner of the core to the store. Called on a
/// thread of the core while it holds a lock, so it only passes the state on
/// and returns: no call into the core, no waiting for the main actor.
final class StoreListener: StateListener {
    private let continuation: AsyncStream<NasState>.Continuation

    init(_ continuation: AsyncStream<NasState>.Continuation) {
        self.continuation = continuation
    }

    func stateChanged(state: NasState) {
        continuation.yield(state)
    }

    /// Ends the stream of the store; later states are dropped.
    func finish() {
        continuation.finish()
    }
}

/// A keychain without entries that stores nothing; for the previews.
private struct NoAccountStore: AccountStore {
    func loadAll() throws -> [StoredAccount] { [] }
    func save(_ stored: StoredAccount) throws {}
    func delete(id: UUID) throws {}
}
