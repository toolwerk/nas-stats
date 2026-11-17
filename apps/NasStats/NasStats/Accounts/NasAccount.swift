import Foundation

/// A NAS the user added, without the password. Stored in the keychain
/// together with the password as a `StoredAccount`.
struct NasAccount: Codable, Identifiable, Sendable, Equatable {
    /// Local id, also the id of the NAS in the core and the keychain account.
    var id: UUID
    var name: String
    /// Vendor as a stable key (`"qnap"`), so an entry of a later vendor
    /// still decodes in an older app version.
    var providerKey: String
    /// `https://host[:port]`, see `NasAddress.normalize`.
    var baseUrl: String
    /// SHA-256 fingerprint the user confirmed, `AB:CD:…`.
    var fingerprint: String
    /// Empty for a vendor with an API key instead of a user (Unraid).
    var user: String
    /// Model from the test query, `nil` if the NAS did not report it.
    var model: String?
    /// Whether the keychain entry syncs over iCloud Keychain.
    var syncsViaICloud: Bool

    static let qnapKey = "qnap"
    static let unraidKey = "unraid"

    /// The vendor for the core; `nil` for a vendor this version cannot
    /// talk to.
    var provider: ProviderKind? {
        switch providerKey {
        case Self.qnapKey: .qnap
        case Self.unraidKey: .unraid
        default: nil
        }
    }

    /// Configuration for the core; `nil` for a vendor this version cannot
    /// talk to.
    var config: NasConfig? {
        guard let provider else { return nil }
        return NasConfig(
            id: id.uuidString,
            provider: provider,
            baseUrl: baseUrl,
            fingerprint: fingerprint
        )
    }
}

/// Content of one keychain entry: the whole NAS and its password.
struct StoredAccount: Codable, Sendable, Equatable {
    var account: NasAccount
    /// Password, or the API key for a vendor without user and password.
    var password: String
}
