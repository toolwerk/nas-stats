import Foundation
import Observation
import os

/// The NAS of the user: loaded from the keychain and registered with the
/// core at start, because the core keeps NAS only in memory.
@MainActor
@Observable
final class NasAccounts {
    @ObservationIgnored let core = NasCore()
    @ObservationIgnored private let keychain: AccountKeychain

    /// Sorted by name.
    private(set) var accounts: [NasAccount] = []
    /// Set when the keychain could not be read; the list is empty then.
    private(set) var loadError: String?

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "accounts")

    init(keychain: AccountKeychain = AccountKeychain()) {
        self.keychain = keychain
        do {
            let stored = try keychain.loadAll()
            for entry in stored {
                register(entry)
            }
            accounts = Self.sorted(stored.map(\.account))
        } catch {
            Self.log.error("Keychain not readable: \(String(describing: error), privacy: .public)")
            loadError = "Die gespeicherten NAS ließen sich nicht aus dem Schlüsselbund lesen."
        }
    }

    /// Stores a NAS after its test query and registers it with the core.
    /// If the core refuses it, the keychain entry is removed again.
    func add(_ stored: StoredAccount) throws {
        guard let config = stored.account.config else {
            throw FfiError.InvalidInput(detail: "Anbieter \(stored.account.providerKey) unbekannt")
        }
        try keychain.save(stored)
        do {
            try core.addNas(config: config, user: stored.account.user, password: stored.password)
        } catch {
            try? keychain.delete(id: stored.account.id)
            throw error
        }
        accounts = Self.sorted(accounts.filter { $0.id != stored.account.id } + [stored.account])
    }

    /// Registers a stored NAS with the core. A failure only logs: the NAS
    /// stays in the list.
    private func register(_ stored: StoredAccount) {
        let id = stored.account.id.uuidString
        guard let config = stored.account.config else {
            Self.log.error("NAS \(id, privacy: .public): unknown provider \(stored.account.providerKey, privacy: .public)")
            return
        }
        do {
            try core.addNas(config: config, user: stored.account.user, password: stored.password)
        } catch {
            Self.log.error("NAS \(id, privacy: .public) not registered: \(String(describing: error), privacy: .public)")
        }
    }

    private static func sorted(_ accounts: [NasAccount]) -> [NasAccount] {
        accounts.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}
