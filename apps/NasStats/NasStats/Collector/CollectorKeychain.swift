import Foundation
import os
import Security
import Synchronization

/// Where the paired collector is kept; `CollectorKeychain` in the app, a
/// memory store in the previews and the tests.
protocol CollectorStore: Sendable {
    /// The paired collector, `nil` without one.
    func load() throws -> StoredCollector?
    func save(_ collector: StoredCollector) throws
    /// Removes the entry; a missing entry is no error.
    func delete() throws
}

/// Keeps the collector as one generic password item: service
/// `net.staude.nasstats.collector`, account `collector`, data = JSON of
/// `StoredCollector`. iCloud Keychain only when the entry asks for it.
struct CollectorKeychain: CollectorStore {
    static let service = "net.staude.nasstats.collector"
    static let account = "collector"

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "keychain")

    /// Attributes every query shares, the same as for the NAS.
    private func base() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: Self.account,
            kSecAttrAccessGroup as String: AppGroup.keychainGroup,
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
        ]
    }

    /// The entry, local or synced. One that does not decode is `nil` with a
    /// log line.
    func load() throws -> StoredCollector? {
        var query = base()
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        query[kSecReturnData as String] = true

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        guard let data = result as? Data,
              let stored = try? JSONDecoder().decode(StoredCollector.self, from: data)
        else {
            Self.log.error("Skipping unreadable collector keychain entry")
            return nil
        }
        return stored
    }

    /// Replaces the entry, also when its iCloud setting changed.
    func save(_ collector: StoredCollector) throws {
        try delete()
        var query = base()
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        query[kSecAttrSynchronizable as String] = collector.syncsViaICloud
        query[kSecValueData as String] = try JSONEncoder().encode(collector)
        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    func delete() throws {
        let status = SecItemDelete(base() as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(status: status)
        }
    }
}

/// Collector store in memory, for the previews and the tests.
final class MemoryCollectorStore: CollectorStore {
    private let item: Mutex<StoredCollector?>
    /// `save` and `delete` throw while set.
    let refuseSave = Mutex(false)

    struct Refused: Error {}

    init(_ stored: StoredCollector? = nil) {
        item = Mutex(stored)
    }

    var stored: StoredCollector? { item.withLock { $0 } }

    func load() throws -> StoredCollector? { stored }

    func save(_ collector: StoredCollector) throws {
        if refuseSave.withLock({ $0 }) { throw Refused() }
        item.withLock { $0 = collector }
    }

    func delete() throws {
        if refuseSave.withLock({ $0 }) { throw Refused() }
        item.withLock { $0 = nil }
    }
}
