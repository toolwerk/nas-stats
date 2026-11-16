import Foundation

/// Kind of problem the settings switch notifications for (#39); several
/// kinds of events of the core share one.
enum NotificationCategory: String, Codable, CaseIterable {
    case alert, disks, volumes, fans, backups, ups, unreachable

    /// Label of the switch in the settings.
    var title: String {
        switch self {
        case .alert: "Warnungen des NAS"
        case .disks: "Platten"
        case .volumes: "Volumes"
        case .fans: "Lüfter"
        case .backups: "Backups"
        case .ups: "USV"
        case .unreachable: "Nicht erreichbar"
        }
    }

    init(_ kind: EventKind) {
        switch kind {
        case .alert: self = .alert
        case .diskHealth, .diskTemperature: self = .disks
        case .volumeHealth, .volumeFill: self = .volumes
        case .fanHealth: self = .fans
        case .backupFailed, .backupOverdue: self = .backups
        case .upsOnBattery: self = .ups
        case .unreachable: self = .unreachable
        }
    }
}

/// Time of day in which notifications wait until its end (#39). Minutes
/// after midnight in the time zone of the device; a start after the end
/// spans midnight, a start equal to the end never holds anything back.
struct QuietHours: Codable, Equatable {
    var enabled = false
    /// 22:00.
    var startMinute = 1320
    /// 07:00.
    var endMinute = 420

    /// Seconds from `date` to the next end, when `date` lies in the quiet
    /// hours; `nil` otherwise. Does not look at `enabled`.
    func delay(at date: Date, calendar: Calendar) -> TimeInterval? {
        guard startMinute != endMinute else { return nil }
        let parts = calendar.dateComponents([.hour, .minute], from: date)
        let minute = (parts.hour ?? 0) * 60 + (parts.minute ?? 0)
        let inside = startMinute < endMinute
            ? minute >= startMinute && minute < endMinute
            : minute >= startMinute || minute < endMinute
        guard inside,
              let end = calendar.nextDate(
                  after: date,
                  matching: DateComponents(hour: endMinute / 60, minute: endMinute % 60, second: 0),
                  matchingPolicy: .nextTime
              )
        else { return nil }
        return end.timeIntervalSince(date)
    }
}

/// Notification settings of the app (#39), saved under `notifications`
/// in the defaults like `AlertThresholds`.
struct NotificationSettings: Codable, Equatable {
    /// Switched off kinds; empty means every kind is on.
    var disabled: Set<NotificationCategory> = []
    var quietHours = QuietHours()

    static let defaultsKey = "notifications"

    func isEnabled(_ category: NotificationCategory) -> Bool {
        !disabled.contains(category)
    }

    /// The stored settings; missing or unreadable ones are the default.
    static func load(from defaults: UserDefaults) -> NotificationSettings {
        guard let data = defaults.data(forKey: defaultsKey),
              let stored = try? JSONDecoder().decode(NotificationSettings.self, from: data)
        else { return NotificationSettings() }
        return stored
    }

    func save(to defaults: UserDefaults) {
        guard let data = try? JSONEncoder().encode(self) else { return }
        defaults.set(data, forKey: Self.defaultsKey)
    }
}
