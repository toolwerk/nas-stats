#if os(iOS)
import UIKit

/// Wakes the iPhone app for the silent push of iCloud after the Mac wrote
/// (#37): it takes over the new records, and the widgets follow through
/// the store.
final class PhoneAppDelegate: NSObject, UIApplicationDelegate {
    /// The store of the app; set by `NasStatsApp` before the first push.
    @MainActor static weak var store: NasStore?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        application.registerForRemoteNotifications()
        return true
    }

    @MainActor
    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any]
    ) async -> UIBackgroundFetchResult {
        guard let store = Self.store else { return .noData }
        return await store.fetchFromCloud() ? .newData : .noData
    }
}
#endif
