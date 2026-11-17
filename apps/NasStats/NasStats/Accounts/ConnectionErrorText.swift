import Foundation

/// German text for an error of the add dialog: from the core, the address
/// check or the keychain.
func connectionErrorText(_ error: any Error, provider: ProviderKind? = nil) -> String {
    if let error = error as? FfiError {
        return text(for: error, provider: provider)
    }
    if let error = error as? NasAddressError {
        return text(for: error)
    }
    if let error = error as? KeychainError {
        return "Die Zugangsdaten ließen sich nicht im Schlüsselbund ablegen "
            + "(Status \(error.status))."
    }
    return "Verbindung fehlgeschlagen: \(error.localizedDescription)"
}

private func text(for error: FfiError, provider: ProviderKind?) -> String {
    if provider == .unraid {
        switch error {
        case .LoginFailed:
            return "Der API-Key ist falsch oder abgelaufen. NAS Stats versucht es "
                + "nicht von selbst erneut."
        case .PermissionDenied:
            return "Dem API-Key fehlen Rechte. Legen Sie einen API-Key mit der Rolle „VIEWER“ an."
        default:
            break
        }
    }
    switch error {
    case .LoginFailed:
        return "Benutzername oder Passwort ist falsch. NAS Stats versucht es "
            + "nicht von selbst erneut, damit das NAS die Adresse nicht sperrt."
    case .SecondFactorRequired:
        return "Das Konto verlangt eine 2-Schritt-Bestätigung. Nehmen Sie ein "
            + "eigenes Konto ohne 2-Schritt-Bestätigung."
    case .IpBlocked:
        return "Das NAS hat diese Adresse gesperrt. Entsperren Sie sie in DSM unter "
            + "Systemsteuerung › Sicherheit › Schutz oder warten Sie die Sperrzeit ab."
    case .PermissionDenied:
        return "Dem Konto fehlen Rechte. Geben Sie ihm die Rolle „System Monitoring“."
    case .Unreachable, .Timeout:
        return "Das NAS ist nicht erreichbar. Prüfen Sie Adresse, Port und Netz."
    case .CertificateChanged:
        return "Das Zertifikat des NAS hat sich geändert. Rufen Sie den "
            + "Fingerabdruck neu ab und prüfen Sie ihn."
    case let .InvalidInput(detail):
        return "Ungültige Angabe: \(detail)"
    case let .Http(status):
        return "Das NAS antwortet mit HTTP-Status \(status)."
    default:
        return "Verbindung fehlgeschlagen: \(error.localizedDescription)"
    }
}

private func text(for error: NasAddressError) -> String {
    switch error {
    case .empty:
        return "Bitte eine Adresse eingeben."
    case .notHTTPS:
        return "NAS Stats verbindet sich nur über HTTPS. Nehmen Sie https:// und den HTTPS-Port."
    case .invalid:
        return "Die Adresse ist ungültig. Erwartet wird z. B. https://192.168.178.20:443."
    }
}
