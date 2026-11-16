import Foundation

/// Where `EventNotifier` sends its notifications; the notification center
/// of the system in the app (`SystemNotificationPoster`), a fake in the
/// tests.
@MainActor
protocol NotificationPoster: AnyObject {
    /// Shows a notification after `delay` seconds (`nil` or 0: at once).
    /// The same `id` replaces an earlier one, pending or delivered.
    func post(id: String, group: String, title: String, body: String, delay: TimeInterval?)
    /// Removes the notifications, pending and delivered.
    func withdraw(ids: [String])
    /// Asks the user once; later calls do nothing.
    func requestAuthorization() async
    /// Whether the user switched notifications off for the app.
    func isDenied() async -> Bool
    /// Whether the system shows notifications of the app now.
    func isAuthorized() async -> Bool
    /// Gives every pending notification for which `keep` holds at that
    /// moment a new delay (`nil`: at once).
    func reschedulePending(delay: TimeInterval?, keep: @escaping @MainActor (String) -> Bool)
}

/// Turns the open events of the core (#10) into notifications (#39): one
/// per event, again only after it ended and began anew. What it notified
/// is saved under `notifiedEvents` in the defaults, so a restart does not
/// notify a lasting problem a second time.
@MainActor
final class EventNotifier {
    /// What the notifier remembers of an open event.
    struct Notified: Codable, Equatable {
        var nasId: String
        var category: NotificationCategory
        /// Already critical when last seen; a rise to critical notifies again.
        var critical: Bool
        /// Whether a notification went out; not while its kind was off.
        var posted: Bool
    }

    static let defaultsKey = "notifiedEvents"

    private let poster: any NotificationPoster
    private let defaults: UserDefaults
    private let clock: () -> Date
    private let calendar: Calendar
    /// Event id → what was notified.
    private var notified: [String: Notified]
    /// Whether the system shows notifications; until then `update` takes in
    /// no new event, so one that began before the permission still
    /// notifies once it is given (`refreshAuthorization`).
    private(set) var authorized = false

    init(
        poster: any NotificationPoster,
        defaults: UserDefaults,
        clock: @escaping () -> Date = Date.init,
        calendar: Calendar = .current
    ) {
        self.poster = poster
        self.defaults = defaults
        self.clock = clock
        self.calendar = calendar
        // Unreadable counts as empty: at worst one notification too many.
        self.notified = defaults.data(forKey: Self.defaultsKey)
            .flatMap { try? JSONDecoder().decode([String: Notified].self, from: $0) } ?? [:]
    }

    /// Id of an event and of its notification: the same event keeps its
    /// start, a new one after a recovery gets a new start.
    static func id(of event: NasEvent) -> String {
        "\(event.nasId)|\(event.kind)|\(event.subject ?? "")|\(event.startedAt)"
    }

    /// Reads the permission of the system again; call it before a query,
    /// the user may change it in the system settings at any time.
    func refreshAuthorization() async {
        authorized = await poster.isAuthorized()
    }

    /// Takes in the open events of one NAS after a query: notifies new
    /// ones and a rise to critical, withdraws those that ended. Without
    /// permission it only withdraws.
    func update(nasId: String, name: String, open: [NasEvent], settings: NotificationSettings) {
        var openIds = Set<String>()
        for event in open {
            let id = Self.id(of: event)
            openIds.insert(id)
            let critical = event.level == .critical
            let enabled = settings.isEnabled(NotificationCategory(event.kind))
            if var known = notified[id] {
                // A rise from warning to critical replaces the notification.
                guard critical, !known.critical, authorized else { continue }
                known.critical = true
                if enabled {
                    post(event, id: id, name: name, settings: settings)
                    known.posted = true
                }
                notified[id] = known
            } else if authorized || !enabled {
                // An event that begins while its kind is off stays silent,
                // also after switching it on; also without permission.
                if enabled {
                    post(event, id: id, name: name, settings: settings)
                }
                notified[id] = Notified(
                    nasId: nasId,
                    category: NotificationCategory(event.kind),
                    critical: critical,
                    posted: enabled
                )
            }
        }
        let ended = notified.filter { $0.value.nasId == nasId && !openIds.contains($0.key) }
        withdraw(Array(ended.keys))
        save()
    }

    /// Follows a change of the settings: a kind switched off withdraws its
    /// notifications, its events stay silent; changed quiet hours move the
    /// pending notifications to their new end, or release them at once.
    func apply(_ settings: NotificationSettings, previous: NotificationSettings) {
        let silenced = notified.filter { $0.value.posted && !settings.isEnabled($0.value.category) }
        if !silenced.isEmpty {
            poster.withdraw(ids: silenced.keys.sorted())
            for id in silenced.keys {
                notified[id]?.posted = false
            }
            save()
        }
        if settings.quietHours != previous.quietHours {
            let delay = settings.quietHours.enabled
                ? settings.quietHours.delay(at: clock(), calendar: calendar)
                : nil
            // Only what is still open: an event may end meanwhile.
            poster.reschedulePending(delay: delay) { [weak self] id in
                self?.notified[id]?.posted == true
            }
        }
    }

    /// Withdraws every notification of a removed NAS and forgets its events.
    func forget(nasId: String) {
        let ids = notified.filter { $0.value.nasId == nasId }.map(\.key)
        guard !ids.isEmpty else { return }
        withdraw(ids)
        save()
    }

    func requestAuthorization() async {
        await poster.requestAuthorization()
    }

    func isDenied() async -> Bool {
        await poster.isDenied()
    }

    private func post(_ event: NasEvent, id: String, name: String, settings: NotificationSettings) {
        let delay = settings.quietHours.enabled
            ? settings.quietHours.delay(at: clock(), calendar: calendar)
            : nil
        poster.post(id: id, group: event.nasId, title: name, body: event.text, delay: delay)
    }

    private func withdraw(_ ids: [String]) {
        guard !ids.isEmpty else { return }
        // Silently: the app does not report "back to normal".
        poster.withdraw(ids: ids.sorted())
        for id in ids {
            notified[id] = nil
        }
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(notified) else { return }
        defaults.set(data, forKey: Self.defaultsKey)
    }
}
