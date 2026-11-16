import Foundation

/// What the tab „Ereignisse“ narrows the list to (#20); the default shows
/// everything.
struct EventFilter: Equatable {
    /// Only this NAS; `nil`: all.
    var nasId: String?
    /// Only this kind of problem; `nil`: all.
    var category: NotificationCategory?
    /// Only events that have not ended.
    var onlyOpen = false
}

/// One event of the list.
struct EventRow: Identifiable, Equatable {
    /// `EventNotifier.id(of:)`, the same as the notification of the event.
    let id: String
    /// Text of the event, e.g. „Platte 3 bei 52 °C“.
    let title: String
    /// „Keller · offen seit 25 min“ or „Keller · erledigt nach 4 min“.
    let detail: String
    /// Time of the start, „13:40“.
    let time: String
    let resolved: Bool
    let level: StatusLevel
    /// SF Symbol of the kind.
    let symbol: String
}

/// Events that began on one day.
struct EventDay: Identifiable, Equatable {
    /// Start of the day, Unix seconds.
    let id: Int64
    /// „Heute“, „Gestern“ or „12. November“.
    let title: String
    let rows: [EventRow]
}

/// Chronicle of the events of the core as the tab shows it: filtered,
/// newest first, grouped by the day they began (#20).
struct EventsContent: Equatable {
    let days: [EventDay]
    /// Whether there are events at all, before the filter; tells „Noch keine
    /// Ereignisse“ from „Keine Ereignisse für diesen Filter“.
    let hasAnyEvents: Bool

    /// - Parameters:
    ///   - names: NAS id → name; an event of an unknown NAS reads
    ///     „Unbekanntes NAS“.
    ///   - now: For „offen seit“ and „Heute“.
    init(
        events: [NasEvent],
        names: [String: String],
        filter: EventFilter = EventFilter(),
        now: Date,
        calendar: Calendar = .current,
        locale: Locale = .current
    ) {
        hasAnyEvents = !events.isEmpty
        let shown = events
            .filter { event in
                (filter.nasId == nil || event.nasId == filter.nasId)
                    && (filter.category == nil || NotificationCategory(event.kind) == filter.category)
                    && (!filter.onlyOpen || event.endedAt == nil)
            }
            .sorted { lhs, rhs in
                lhs.startedAt != rhs.startedAt
                    ? lhs.startedAt > rhs.startedAt
                    : EventNotifier.id(of: lhs) < EventNotifier.id(of: rhs)
            }

        let timeFormat = DateFormatter()
        timeFormat.locale = locale
        timeFormat.timeZone = calendar.timeZone
        timeFormat.dateFormat = "HH:mm"
        let nowSeconds = Int64(now.timeIntervalSince1970)

        var days: [EventDay] = []
        for event in shown {
            let start = Date(timeIntervalSince1970: TimeInterval(event.startedAt))
            let day = calendar.startOfDay(for: start)
            let dayId = Int64(day.timeIntervalSince1970)
            let row = Self.row(
                event,
                name: names[event.nasId] ?? "Unbekanntes NAS",
                time: timeFormat.string(from: start),
                now: nowSeconds
            )
            if let last = days.last, last.id == dayId {
                days[days.count - 1] = EventDay(id: dayId, title: last.title, rows: last.rows + [row])
            } else {
                days.append(EventDay(
                    id: dayId,
                    title: Self.title(of: day, now: now, calendar: calendar, locale: locale),
                    rows: [row]
                ))
            }
        }
        self.days = days
    }

    private static func row(_ event: NasEvent, name: String, time: String, now: Int64) -> EventRow {
        let resolved = event.endedAt != nil
        let seconds = max(0, (event.endedAt ?? now) - event.startedAt)
        let duration = StatusFormat.uptime(UInt64(seconds))
        return EventRow(
            id: EventNotifier.id(of: event),
            title: event.text,
            detail: "\(name) · " + (resolved ? "erledigt nach \(duration)" : "offen seit \(duration)"),
            time: time,
            resolved: resolved,
            level: level(of: event),
            symbol: NotificationCategory(event.kind).symbol
        )
    }

    /// Colour of an event: „nicht erreichbar“ grey, else its level; also
    /// for the event line of the Mac popover (#28).
    static func level(of event: NasEvent) -> StatusLevel {
        NotificationCategory(event.kind) == .unreachable ? .offline : StatusLevel(event.level)
    }

    private static func title(of day: Date, now: Date, calendar: Calendar, locale: Locale) -> String {
        let today = calendar.startOfDay(for: now)
        if day == today { return "Heute" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: today), day == yesterday {
            return "Gestern"
        }
        // The year only for a day outside the current one.
        let sameYear = calendar.component(.year, from: day) == calendar.component(.year, from: now)
        let format = DateFormatter()
        format.locale = locale
        format.timeZone = calendar.timeZone
        format.setLocalizedDateFormatFromTemplate(sameYear ? "dMMMM" : "dMMMMy")
        return format.string(from: day)
    }
}

private extension StatusLevel {
    init(_ health: Health) {
        switch health {
        case .ok: self = .ok
        case .warning: self = .warning
        case .critical: self = .critical
        }
    }
}

extension NotificationCategory {
    /// SF Symbol of the kind in the list of events.
    var symbol: String {
        switch self {
        case .alert: "exclamationmark.bubble"
        case .disks: "internaldrive"
        case .volumes: "externaldrive"
        case .fans: "fan"
        case .backups: "clock.arrow.circlepath"
        case .ups: "powerplug"
        case .unreachable: "wifi.slash"
        }
    }
}
