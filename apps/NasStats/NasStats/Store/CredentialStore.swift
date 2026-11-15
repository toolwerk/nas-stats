import Foundation
import Synchronization

/// User name and password of one NAS account.
struct Credentials: Sendable, Equatable {
    var user: String
    var password: String
}

/// Where the store keeps the passwords. The NAS list itself is saved
/// without them (`NasList`); the keychain version follows with #14.
protocol CredentialStore: Sendable {
    /// Credentials of the NAS, `nil` when none are saved.
    func load(nasId: String) throws -> Credentials?
    func save(_ credentials: Credentials, nasId: String) throws
    /// Removes the credentials; no error when none are saved.
    func delete(nasId: String) throws
}

/// Keeps the credentials in memory only, until the app quits. Default
/// until the keychain store of #14 exists; after a restart every NAS shows
/// "Passwort fehlt".
final class InMemoryCredentialStore: CredentialStore {
    private let storage = Mutex<[String: Credentials]>([:])

    func load(nasId: String) throws -> Credentials? {
        storage.withLock { $0[nasId] }
    }

    func save(_ credentials: Credentials, nasId: String) throws {
        storage.withLock { $0[nasId] = credentials }
    }

    func delete(nasId: String) throws {
        _ = storage.withLock { $0.removeValue(forKey: nasId) }
    }
}
