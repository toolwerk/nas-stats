import Foundation
import os

/// One record of the transfer from the Mac to the iPhone (#37), without
/// CloudKit types so it can be tested without a network:
///
/// - `NasState`, name `state-<nasId>`: the last snapshot as the core saves
///   it (`payload`, `observedAt`) and the raw samples of the last 24 hours
///   as an asset (`recent`).
/// - `NasDay`, name `day-<nasId>-<dayStart>`: hourly and daily values,
///   events and backup runs of one UTC day (`payload`, `day`).
///
/// Never credentials: those stay in the keychain.
struct CloudRecord: Sendable, Equatable {
    enum Kind: String, Sendable {
        case state = "NasState"
        case day = "NasDay"
    }

    /// Version of the records this build writes and reads.
    static let version: Int64 = 1
    /// A field of a record holds at most 1 MB; `payload` stays under it.
    static let maxPayload = 900_000

    var kind: Kind
    var nasId: String
    /// `observedAt` of a state, start of the day of a day; Unix seconds UTC.
    var time: Int64
    /// When the Mac wrote it, Unix seconds UTC.
    var writtenAt: Int64
    var version: Int64 = Self.version
    /// JSON of the core: `savedState` or `exportDay`.
    var payload: Data
    /// JSON of `exportRecent`, only of a state.
    var recent: Data?

    /// Name of the record in the zone.
    var name: String {
        switch kind {
        case .state: "state-\(nasId)"
        case .day: "day-\(nasId)-\(time)"
        }
    }

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "cloud")

    /// The state of a NAS; `nil` when the snapshot is too large for a field.
    static func state(nasId: String, observedAt: Int64, payload: String, recent: String?, writtenAt: Int64) -> CloudRecord? {
        let record = CloudRecord(
            kind: .state,
            nasId: nasId,
            time: observedAt,
            writtenAt: writtenAt,
            payload: Data(payload.utf8),
            recent: recent.map { Data($0.utf8) }
        )
        return record.fits ? record : nil
    }

    /// One day of a NAS; `nil` when it is too large for a field.
    static func day(nasId: String, day: Int64, payload: String, writtenAt: Int64) -> CloudRecord? {
        let record = CloudRecord(
            kind: .day,
            nasId: nasId,
            time: day,
            writtenAt: writtenAt,
            payload: Data(payload.utf8)
        )
        return record.fits ? record : nil
    }

    private var fits: Bool {
        guard payload.count <= Self.maxPayload else {
            Self.log.error("Record too large, skipped: \(name, privacy: .public), \(payload.count) bytes")
            return false
        }
        return true
    }

    /// `payload` as text for the core.
    var payloadText: String? { String(data: payload, encoding: .utf8) }

    /// `recent` as text for the core.
    var recentText: String? { recent.flatMap { String(data: $0, encoding: .utf8) } }
}
