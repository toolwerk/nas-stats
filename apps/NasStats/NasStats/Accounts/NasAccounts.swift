import Foundation
import Observation
import os

/// Where the accounts are kept; `AccountKeychain` in the app, a memory
/// store in the tests.
protocol AccountStore: Sendable {
    func loadAll() throws -> [StoredAccount]
    func save(_ stored: StoredAccount) throws
    func delete(id: UUID) throws
}

extension AccountKeychain: AccountStore {}

/// The NAS of the user: loaded from the keychain and registered with the
/// core at start, because the core keeps NAS only in memory. `NasStore`
/// queries them, holds their state and hands the core the last snapshot
/// of an earlier launch (`LastStates`).
@MainActor
@Observable
final class NasAccounts {
    @ObservationIgnored let core: any NasCoreProtocol
    @ObservationIgnored private let keychain: any AccountStore

    /// Sorted by name.
    private(set) var accounts: [NasAccount] = []
    /// Set when the keychain could not be read; the list is empty then.
    private(set) var loadError: String?
    /// Why a stored NAS is not registered with the core (unknown provider,
    /// configuration refused). The core does not query such a NAS.
    private(set) var registrationErrors: [UUID: String] = [:]

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "accounts")

    init(keychain: any AccountStore = AccountKeychain(), core: any NasCoreProtocol = NasCore()) {
        self.keychain = keychain
        self.core = core
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

    /// Whether the core knows the NAS.
    func isRegistered(_ id: UUID) -> Bool {
        accounts.contains { $0.id == id } && registrationErrors[id] == nil
    }

    /// Stores a NAS after its test query and registers it with the core.
    /// If the core refuses it, the keychain entry is removed again.
    func add(_ stored: StoredAccount) throws {
        let config = try Self.config(of: stored.account)
        try keychain.save(stored)
        do {
            try core.addNas(config: config, user: stored.account.user, password: stored.password)
        } catch {
            try? keychain.delete(id: stored.account.id)
            throw error
        }
        registrationErrors[stored.account.id] = nil
        accounts = Self.sorted(accounts.filter { $0.id != stored.account.id } + [stored.account])
    }

    /// Removes the NAS from the keychain and the core. Throws when the
    /// keychain entry cannot be removed; the NAS stays then.
    func remove(id: UUID) async throws {
        try keychain.delete(id: id)
        accounts.removeAll { $0.id == id }
        registrationErrors[id] = nil
        // The core only throws `UnknownNas` (not registered) or an internal
        // error; the NAS is gone from the keychain either way.
        try? await core.removeNas(nasId: id.uuidString)
    }

    /// New user name and password for a stored NAS, e.g. after a refused
    /// login: lifts a block in the core, or registers a NAS the core refused
    /// at start. When the core refuses, the old keychain entry is put back.
    func updateCredentials(id: UUID, user: String, password: String) async throws {
        guard let current = accounts.first(where: { $0.id == id }) else {
            throw FfiError.UnknownNas(id: id.uuidString)
        }
        var account = current
        account.user = user
        let config = try Self.config(of: account)
        // A keychain that cannot be read stops here, so a failure below
        // never deletes an entry it could not see.
        let old = try keychain.loadAll().first { $0.account.id == id }
        try keychain.save(StoredAccount(account: account, password: password))
        do {
            if isRegistered(id) {
                try await core.updateCredentials(nasId: id.uuidString, user: user, password: password)
            } else {
                do {
                    try core.addNas(config: config, user: user, password: password)
                } catch {
                    registrationErrors[id] = connectionErrorText(error)
                    throw error
                }
            }
        } catch {
            if let old {
                try? keychain.save(old)
            } else {
                try? keychain.delete(id: id)
            }
            throw error
        }
        registrationErrors[id] = nil
        accounts = Self.sorted(accounts.map { $0.id == id ? account : $0 })
    }

    /// New name for a stored NAS, in the keychain and the list; the core
    /// does not know names. Throws `InvalidInput` for an empty name.
    func rename(id: UUID, name: String) throws {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else {
            throw FfiError.InvalidInput(detail: "Der Name darf nicht leer sein.")
        }
        var stored = try storedAccount(id)
        stored.account.name = name
        try keychain.save(stored)
        accounts = Self.sorted(accounts.map { $0.id == id ? stored.account : $0 })
    }

    /// Moves the keychain entry of a NAS to or from iCloud Keychain: it is
    /// stored again with the new setting. The core does not know the
    /// setting, so it is not touched. When the keychain refuses, the old
    /// entry is put back and the list stays as it was.
    func setSyncsViaICloud(id: UUID, syncs: Bool) throws {
        let old = try storedAccount(id)
        guard old.account.syncsViaICloud != syncs else { return }
        var stored = old
        stored.account.syncsViaICloud = syncs
        do {
            try keychain.save(stored)
        } catch {
            try? keychain.save(old)
            throw error
        }
        accounts = Self.sorted(accounts.map { $0.id == id ? stored.account : $0 })
    }

    /// Starts a blocked NAS again with its stored user and password ("Erneut
    /// versuchen"); see `updateCredentials`.
    func resume(id: UUID) async throws {
        let stored = try storedAccount(id)
        try await updateCredentials(id: id, user: stored.account.user, password: stored.password)
    }

    /// Trusts a new certificate (#76): stores the fingerprint and registers
    /// the NAS again with the core, which replaces the old entry and lifts
    /// its block. When the core refuses, the old entry is put back.
    func confirmFingerprint(id: UUID, fingerprint: String) async throws {
        let old = try storedAccount(id)
        var stored = old
        stored.account.fingerprint = fingerprint
        let config = try Self.config(of: stored.account)
        try keychain.save(stored)
        do {
            try core.addNas(config: config, user: stored.account.user, password: stored.password)
        } catch {
            try? keychain.save(old)
            registrationErrors[id] = connectionErrorText(error)
            throw error
        }
        registrationErrors[id] = nil
        accounts = Self.sorted(accounts.map { $0.id == id ? stored.account : $0 })
    }

    /// The keychain entry of a NAS in the list, with its password.
    private func storedAccount(_ id: UUID) throws -> StoredAccount {
        guard accounts.contains(where: { $0.id == id }),
              let stored = try keychain.loadAll().first(where: { $0.account.id == id })
        else {
            throw FfiError.UnknownNas(id: id.uuidString)
        }
        return stored
    }

    /// Registers a stored NAS with the core. A failure stays in
    /// `registrationErrors`: the NAS stays in the list.
    private func register(_ stored: StoredAccount) {
        let id = stored.account.id
        do {
            let config = try Self.config(of: stored.account)
            try core.addNas(config: config, user: stored.account.user, password: stored.password)
        } catch {
            Self.log.error("NAS \(id.uuidString, privacy: .public) not registered: \(String(describing: error), privacy: .public)")
            registrationErrors[id] = connectionErrorText(error)
        }
    }

    private static func config(of account: NasAccount) throws -> NasConfig {
        guard let config = account.config else {
            throw FfiError.InvalidInput(detail: "Anbieter \(account.providerKey) unbekannt")
        }
        return config
    }

    private static func sorted(_ accounts: [NasAccount]) -> [NasAccount] {
        accounts.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}
