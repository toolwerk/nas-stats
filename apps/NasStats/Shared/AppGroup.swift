import Foundation

/// Shared containers of the app and its widget.
enum AppGroup {
    /// App Group for the widget snapshot. macOS needs the team prefix,
    /// iOS the `group.` prefix (project.yml).
    #if os(macOS)
    static let id = "7N2W4T6UPV.net.staude.nasstats"
    #else
    static let id = "group.net.staude.nasstats"
    #endif

    /// Keychain access group for the NAS accounts, the same on both platforms.
    static let keychainGroup = "7N2W4T6UPV.net.staude.nasstats"
}
