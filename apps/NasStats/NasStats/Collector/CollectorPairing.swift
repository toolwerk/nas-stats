import Foundation

/// The collector on the NAS (#46, #124) the app is paired with: address,
/// pinned certificate and key, plus the choices of the user. Kept as one
/// keychain entry (`CollectorKeychain`); the progress of the import is
/// per device and lives in the defaults (`CollectorSync`).
struct StoredCollector: Codable, Sendable, Equatable {
    /// `https://host:port`, see `CollectorAddress.normalize`.
    var baseUrl: String
    /// SHA-256 fingerprint the user confirmed, `AB:CD:…`.
    var fingerprint: String
    /// Key of the collector from `nas-collector pairing`.
    var token: String
    /// NAS of the collector the user assigned by hand: collector NAS id →
    /// id of the NAS in the app. Automatic assignments are not stored.
    var assignments: [String: UUID] = [:]
    /// NAS of the collector the user chose not to take over.
    var ignored: Set<String> = []
    /// Whether the keychain entry syncs over iCloud Keychain; follows
    /// "Zugänge über iCloud-Schlüsselbund teilen".
    var syncsViaICloud: Bool
}

/// Checks and normalizes the address of a collector.
enum CollectorAddress {
    /// Port of the collector when the address names none.
    static let defaultPort = 8743

    /// Like `NasAddress.normalize`; without a port the collector's 8743.
    static func normalize(_ input: String) -> Result<String, NasAddressError> {
        NasAddress.normalize(input).flatMap { address in
            guard var parts = URLComponents(string: address) else { return .failure(.invalid) }
            if parts.port == nil {
                parts.port = defaultPort
            }
            guard let string = parts.string else { return .failure(.invalid) }
            return .success(string)
        }
    }
}

/// Whether two fingerprints name the same certificate: colons, spaces and
/// case do not count.
func sameFingerprint(_ lhs: String, _ rhs: String) -> Bool {
    let plain = { (value: String) in
        value.filter { $0 != ":" && !$0.isWhitespace }.lowercased()
    }
    return !plain(lhs).isEmpty && plain(lhs) == plain(rhs)
}

/// German text for an error of the collector.
func collectorErrorText(_ error: any Error) -> String {
    switch error as? FfiError {
    case .Unreachable?, .Timeout?:
        return String(localized: "Container nicht erreichbar")
    case .LoginFailed?:
        return String(localized: "Schlüssel abgelehnt – neu koppeln")
    case .CertificateChanged?:
        return String(localized: "Zertifikat des Containers geändert – neu koppeln")
    default:
        return connectionErrorText(error)
    }
}

/// Error of the import of one NAS of the collector.
func collectorImportErrorText(_ error: any Error) -> String {
    if case let .Http(status)? = error as? FfiError, status == 404 {
        return String(localized: "Nicht mehr im Container")
    }
    return collectorErrorText(error)
}

/// "zuletzt 14:05" today, else with the day.
func collectorLastSyncText(_ date: Date, now: Date = .now) -> String {
    let time = Calendar.current.isDate(date, inSameDayAs: now)
        ? date.formatted(date: .omitted, time: .shortened)
        : date.formatted(date: .abbreviated, time: .shortened)
    return String(localized: "zuletzt \(time)")
}
