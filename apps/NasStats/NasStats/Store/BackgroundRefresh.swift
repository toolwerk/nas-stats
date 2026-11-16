#if os(iOS)
import BackgroundTasks
import Foundation
import os

/// The occasional query in the background (`BGAppRefreshTask`): iOS decides
/// when and whether it runs, so the app promises no frequency.
enum BackgroundRefresh {
    /// Also under `BGTaskSchedulerPermittedIdentifiers` in project.yml.
    static let identifier = "net.staude.nasstats.refresh"

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "background-refresh")

    /// Asks iOS for a run at the earliest after the query interval of the
    /// settings, at least 5 minutes; replaces an earlier request. A refusal
    /// (e.g. in the simulator) is only logged.
    static func schedule(intervalMinutes: Int, now: Date = .now) async {
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = now.addingTimeInterval(TimeInterval(max(300, intervalMinutes * 60)))
        do {
            try await BGTaskScheduler.shared.submitTaskRequest(request)
        } catch {
            log.error("Background refresh not scheduled: \(String(describing: error), privacy: .public)")
        }
    }
}
#endif
