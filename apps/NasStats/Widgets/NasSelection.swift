import AppIntents
import WidgetKit

/// One choice in the widget settings (#26): „Automatisch“ or a NAS of the
/// widget snapshot.
struct NasEntity: AppEntity {
    static let typeDisplayRepresentation = TypeDisplayRepresentation(name: "NAS")
    static let defaultQuery = NasQuery()

    /// The NAS with the worst state.
    static let automatic = NasEntity(
        id: WidgetSnapshot.automaticId, name: "Automatisch", subtitle: "Schlechtester Zustand"
    )

    let id: String
    let name: String
    let subtitle: String?

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", subtitle: subtitle.map { "\($0)" })
    }
}

/// Lists the NAS of the widget snapshot the app wrote (#23).
struct NasQuery: EntityQuery {
    func suggestedEntities() async throws -> [NasEntity] {
        var entities = [NasEntity.automatic]
        guard let snapshot = WidgetSnapshot.load(from: WidgetSnapshot.fileURL) else { return entities }
        entities += snapshot.nas.map { nas in
            let parts = [snapshot.demo ? "Demo" : nil, nas.vendor].compactMap { $0 }
            return NasEntity(
                id: nas.id, name: nas.name,
                subtitle: parts.isEmpty ? nil : parts.joined(separator: " · ")
            )
        }
        return entities
    }

    /// Unknown ids drop out; the widget then shows the worst NAS.
    func entities(for identifiers: [String]) async throws -> [NasEntity] {
        let all = try await suggestedEntities()
        return identifiers.compactMap { id in all.first { $0.id == id } }
    }

    /// A new widget starts on „Automatisch“.
    func defaultResult() async -> NasEntity? { .automatic }
}

/// Settings of the NAS widgets: which NAS they show. The volume of the fill
/// level widget follows with #38.
struct SelectNasIntent: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "NAS wählen"
    static let description = IntentDescription("Welches NAS das Widget zeigt.")

    @Parameter(title: "NAS")
    var nas: NasEntity?

    init() {}
}
