import Foundation
import os
import Security

/// A keychain call failed with this `OSStatus`.
struct KeychainError: Error, Equatable, Sendable {
    let status: OSStatus
}

/// Stores each NAS as one generic password item: service
/// `net.staude.nasstats.nas`, account = id, data = JSON of `StoredAccount`.
/// iCloud Keychain only when the account asks for it.
struct AccountKeychain: Sendable {
    static let service = "net.staude.nasstats.nas"

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "keychain")

    /// Attributes every query shares.
    private func base() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccessGroup as String: AppGroup.keychainGroup,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    /// Every stored NAS, local and synced. An entry that does not decode is
    /// skipped with a log line.
    func loadAll() throws -> [StoredAccount] {
        var query = base()
        query[kSecAttrSynchronizable as String] = kSecAttrSynchronizableAny
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        query[kSecReturnAttributes as String] = true
        query[kSecReturnData as String] = true

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        guard let items = result as? [[String: Any]] else { return [] }

        let decoder = JSONDecoder()
        return items.compactMap { item in
            guard let data = item[kSecValueData as String] as? Data,
                  let stored = try? decoder.decode(StoredAccount.self, from: data)
            else {
                let account = item[kSecAttrAccount as String] as? String ?? "?"
                Self.log.error("Skipping unreadable keychain entry \(account, privacy: .public)")
                return nil
            }
            return stored
        }
    }

    /// The query that adds the entry of the account; `kSecAttrSynchronizable`
    /// follows the iCloud setting of the account.
    func addQuery(for stored: StoredAccount) throws -> [String: Any] {
        var query = base()
        query[kSecAttrAccount as String] = stored.account.id.uuidString
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        query[kSecAttrSynchronizable as String] = stored.account.syncsViaICloud
        query[kSecValueData as String] = try JSONEncoder().encode(stored)
        return query
    }

    /// The query that removes the entry, local or synced.
    func deleteQuery(id: UUID) -> [String: Any] {
        var query = base()
        query[kSecAttrAccount as String] = id.uuidString
        query[kSecAttrSynchronizable as String] = kSecAttrSynchronizableAny
        return query
    }

    /// Replaces the entry of the account, also when its iCloud setting
    /// changed.
    func save(_ stored: StoredAccount) throws {
        try delete(id: stored.account.id)
        let status = SecItemAdd(try addQuery(for: stored) as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    /// Removes the entry, local or synced; a missing entry is no error.
    func delete(id: UUID) throws {
        let status = SecItemDelete(deleteQuery(id: id) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(status: status)
        }
    }
}
