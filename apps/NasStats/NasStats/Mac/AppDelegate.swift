#if os(macOS)
import AppKit

/// Start of the Mac app: the store and the menu bar symbol live as long as
/// the app, so the queries run without an open menu or window (#27).
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var store: NasStore?
    private var statusItem: StatusItemController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let store = NasStore.live()
        self.store = store
        statusItem = StatusItemController(store: store)
        // The 5-minute loop of the store; the planner of #11 replaces it with #67.
        Task { await store.start() }
    }
}
#endif
