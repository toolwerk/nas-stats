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

    /// Registers the app as a login item or removes it.
    static func toggle() {
        setEnabled(!isEnabled)
    }

    /// Registers the app as a login item or removes it, as asked; does
    /// nothing when the system already has that state. When the system wants
    /// the user to approve it, the login items settings open.
    static func setEnabled(_ enabled: Bool) {
        let service = SMAppService.mainApp
        guard enabled != (service.status == .enabled) else { return }
        do {
            if enabled {
                try service.register()
            } else {
                try service.unregister()
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
