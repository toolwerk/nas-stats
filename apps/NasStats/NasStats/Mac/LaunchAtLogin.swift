#if os(macOS)
import OSLog
import ServiceManagement

/// "Bei Anmeldung starten": the app as a login item of the system (#27).
@MainActor
enum LaunchAtLogin {
    private static let logger = Logger(subsystem: "net.staude.nasstats", category: "launch-at-login")

    /// State of the system, not a stored setting.
    static var isEnabled: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// Registers the app as a login item or removes it. When the system
    /// wants the user to approve it, the login items settings open.
    static func toggle() {
        let service = SMAppService.mainApp
        do {
            if service.status == .enabled {
                try service.unregister()
            } else {
                try service.register()
            }
        } catch {
            logger.error("Login item not changed: \(error.localizedDescription, privacy: .public)")
        }
        if service.status == .requiresApproval {
            SMAppService.openSystemSettingsLoginItems()
        }
    }
}
#endif
