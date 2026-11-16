import Foundation
import os
import UserNotifications

/// Notifications through the notification center of the system (#39).
/// Also shows them while the app is in front (Mac: menu bar app).
@MainActor
final class SystemNotificationPoster: NSObject, NotificationPoster, UNUserNotificationCenterDelegate {
    private nonisolated static let log = Logger(subsystem: "net.staude.nasstats", category: "notifications")

    private let center = UNUserNotificationCenter.current()

    override init() {
        super.init()
        center.delegate = self
    }

    func post(id: String, group: String, title: String, body: String, delay: TimeInterval?) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.threadIdentifier = group
        content.sound = .default
        let trigger = delay.flatMap { $0 > 0 ? UNTimeIntervalNotificationTrigger(timeInterval: $0, repeats: false) : nil }
        let request = UNNotificationRequest(identifier: id, content: content, trigger: trigger)
        center.add(request) { error in
            if let error {
                Self.log.error("Notification not added: \(String(describing: error), privacy: .public)")
            }
        }
    }

    func withdraw(ids: [String]) {
        center.removePendingNotificationRequests(withIdentifiers: ids)
        center.removeDeliveredNotifications(withIdentifiers: ids)
    }

    /// The running `reschedulePending`; a newer call replaces it.
    private var rescheduling: Task<Void, Never>?

    func reschedulePending(delay: TimeInterval?, keep: @escaping @MainActor (String) -> Bool) {
        rescheduling?.cancel()
        rescheduling = Task {
            let pending = await center.pendingNotificationRequests()
            guard !Task.isCancelled else { return }
            let trigger = delay.flatMap {
                $0 > 0 ? UNTimeIntervalNotificationTrigger(timeInterval: $0, repeats: false) : nil
            }
            // No suspension from here on: a withdrawal cannot slip between
            // the check and the new request.
            for request in pending where keep(request.identifier) {
                // The same id replaces the pending request.
                center.add(UNNotificationRequest(
                    identifier: request.identifier, content: request.content, trigger: trigger
                )) { error in
                    if let error {
                        Self.log.error("Notification not moved: \(String(describing: error), privacy: .public)")
                    }
                }
            }
        }
    }

    func requestAuthorization() async {
        do {
            _ = try await center.requestAuthorization(options: [.alert, .sound])
        } catch {
            Self.log.error("Notifications not authorized: \(String(describing: error), privacy: .public)")
        }
    }

    func isDenied() async -> Bool {
        await center.notificationSettings().authorizationStatus == .denied
    }

    func isAuthorized() async -> Bool {
        switch await center.notificationSettings().authorizationStatus {
        case .authorized, .provisional: true
        default: false
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }
}
