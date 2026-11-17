import Foundation

/// What the add dialog and the access settings show per vendor (#36).
extension ProviderKind {
    /// Vendors the core can query and the dialog offers; the others show as
    /// "folgt" until their provider delivers values.
    var isSelectable: Bool {
        switch self {
        case .qnap, .unraid: true
        case .synology, .trueNas: false
        }
    }

    /// Display name, as on the card badge.
    var displayName: String {
        switch self {
        case .qnap: "QNAP"
        case .synology: "Synology"
        case .trueNas: "TrueNAS"
        case .unraid: "Unraid"
        }
    }

    /// Key of `NasAccount.providerKey`; `nil` for a vendor without access
    /// from this version.
    var accountKey: String? {
        switch self {
        case .qnap: NasAccount.qnapKey
        case .unraid: NasAccount.unraidKey
        case .synology, .trueNas: nil
        }
    }

    /// Whether the access has a user name (not the case for an API key).
    var usesUser: Bool { self != .unraid }

    /// Label of the secret field.
    var secretLabel: String { usesUser ? "Passwort" : "API-Key" }

    /// Hint under the credentials.
    var accessHint: String {
        switch self {
        case .unraid:
            "Empfohlen: eigener API-Key mit der Rolle „VIEWER“ (ab Unraid 7.2), "
                + "zu finden unter Einstellungen › Management Access › API Keys. "
                + "NAS Stats liest nur."
        default:
            "Empfohlen: eigenes Konto mit der Rolle „System Monitoring“ (ab QTS 5.1), "
                + "ohne 2-Schritt-Bestätigung. NAS Stats liest nur."
        }
    }

    /// Where to compare the certificate fingerprint.
    var certificateHint: String {
        switch self {
        case .unraid:
            "Prüfen Sie den Fingerabdruck im Browser auf der Unraid-Oberfläche "
                + "(Schloss-Symbol › Zertifikat)."
        case .qnap:
            "Prüfen Sie den Fingerabdruck in QTS unter Systemsteuerung › Sicherheit › Zertifikat."
        case .synology, .trueNas:
            "Prüfen Sie den Fingerabdruck in der Oberfläche des NAS."
        }
    }

    /// Text after a refused secret: the access is not sent again.
    var refusedHint: String {
        usesUser
            ? "Das NAS hat diese Zugangsdaten abgelehnt. Ändern Sie Benutzer oder Passwort, um es erneut zu versuchen."
            : "Das NAS hat diesen API-Key abgelehnt. Ändern Sie den API-Key, um es erneut zu versuchen."
    }
}
