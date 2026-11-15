import Foundation

/// A NAS as the app saves it: everything the core needs to register it,
/// without the password (`CredentialStore`).
struct StoredNas: Codable, Equatable, Sendable, Identifiable {
    var id: String
    var provider: ProviderKind
    /// `https://host[:port]`.
    var baseUrl: String
    /// Pinned SHA-256 fingerprint of the certificate, `AB:CD:…`.
    var fingerprint: String
    /// Name chosen by the user; `nil` uses the name the NAS reports.
    var name: String?

    init(id: String, provider: ProviderKind, baseUrl: String, fingerprint: String, name: String? = nil) {
        self.id = id
        self.provider = provider
        self.baseUrl = baseUrl
        self.fingerprint = fingerprint
        self.name = name
    }

    /// What `NasCore.addNas` takes.
    var config: NasConfig {
        NasConfig(id: id, provider: provider, baseUrl: baseUrl, fingerprint: fingerprint)
    }

    private enum CodingKeys: String, CodingKey {
        case id, provider, baseUrl, fingerprint, name
    }

    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let providerName = try container.decode(String.self, forKey: .provider)
        guard let provider = Self.provider(named: providerName) else {
            throw DecodingError.dataCorruptedError(
                forKey: .provider, in: container,
                debugDescription: "Unbekannter Anbieter \(providerName)"
            )
        }
        self.init(
            id: try container.decode(String.self, forKey: .id),
            provider: provider,
            baseUrl: try container.decode(String.self, forKey: .baseUrl),
            fingerprint: try container.decode(String.self, forKey: .fingerprint),
            name: try container.decodeIfPresent(String.self, forKey: .name)
        )
    }

    func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(Self.name(of: provider), forKey: .provider)
        try container.encode(baseUrl, forKey: .baseUrl)
        try container.encode(fingerprint, forKey: .fingerprint)
        try container.encodeIfPresent(name, forKey: .name)
    }

    static func name(of provider: ProviderKind) -> String {
        switch provider {
        case .qnap: "qnap"
        case .synology: "synology"
        case .trueNas: "truenas"
        case .unraid: "unraid"
        }
    }

    static func provider(named name: String) -> ProviderKind? {
        switch name {
        case "qnap": .qnap
        case "synology": .synology
        case "truenas": .trueNas
        case "unraid": .unraid
        default: nil
        }
    }
}

/// The saved NAS list: JSON under `nasList` in the given defaults (in the
/// app those of the App Group). Broken JSON reads as an empty list, an
/// entry with an unknown provider is skipped.
struct NasList {
    static let key = "nasList"

    let defaults: UserDefaults

    func load() -> [StoredNas] {
        guard let data = defaults.data(forKey: Self.key),
              let entries = try? JSONDecoder().decode([Lenient].self, from: data)
        else { return [] }
        return entries.compactMap(\.nas)
    }

    func save(_ list: [StoredNas]) {
        guard let data = try? JSONEncoder().encode(list) else { return }
        defaults.set(data, forKey: Self.key)
    }

    /// One entry that may fail to decode without failing the whole list.
    private struct Lenient: Decodable {
        let nas: StoredNas?

        init(from decoder: any Decoder) throws {
            nas = try? StoredNas(from: decoder)
        }
    }
}
