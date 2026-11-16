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
        // The planner of the core queries from now on.
        store.start()
    }

    /// ⌘, of the app menu (#28).
    func showSettings() {
        statusItem?.showSettings()
    }
}
#endif
