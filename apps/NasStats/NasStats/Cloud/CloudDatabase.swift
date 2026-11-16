import CloudKit
import Foundation
import os

/// State of the iCloud account, as far as the transfer (#37) needs it.
enum CloudAccountStatus: Sendable, Equatable {
    case available
    /// Not signed in to iCloud, or iCloud Drive is off.
    case noAccount
    /// Restricted by the device administrator or temporarily unavailable.
    case unavailable
}

/// Errors of the database that the transfer handles itself.
enum CloudDatabaseError: Error, Equatable {
    /// The change token is too old; fetch again without one.
    case tokenExpired
    /// The zone does not exist (yet): no Mac wrote to it.
    case zoneNotFound
}

/// What changed in the zone since a token.
struct CloudChanges: Sendable {
    var records: [CloudRecord]
    /// Token for the next fetch.
    var token: Data?
}

/// The private iCloud database of the transfer (#37), as the transfer sees
/// it: `CloudKitDatabase` in the app, a fake in the tests, which never
/// create a `CKContainer` (the test bundle has no entitlements).
protocol CloudDatabase: Sendable {
    func accountStatus() async throws -> CloudAccountStatus
    /// Creates the zone when it is missing.
    func ensureZone() async throws
    /// Saves the records, replacing those of the same name.
    func save(_ records: [CloudRecord]) async throws
    /// Records changed since `token` (`nil`: all of them).
    func changes(since token: Data?) async throws -> CloudChanges
    /// Deletes the zone with every record in it.
    func deleteZone() async throws
    /// Asks for a silent push after every change of the zone.
    func subscribe() async throws
}

/// The private database of the container `iCloud.net.staude.nasstats`,
/// zone `NasStats`.
struct CloudKitDatabase: CloudDatabase {
    static let containerId = "iCloud.net.staude.nasstats"
    static let zoneName = "NasStats"
    static let subscriptionId = "nasstats-changed"
    /// A request may carry about 2 MB; a day is up to some 100 KB, so ten
    /// records stay under it.
    static let blockSize = 10

    private static let log = Logger(subsystem: "net.staude.nasstats", category: "cloud")

    private var container: CKContainer { CKContainer(identifier: Self.containerId) }
    private var database: CKDatabase { container.privateCloudDatabase }
    private var zoneID: CKRecordZone.ID {
        CKRecordZone.ID(zoneName: Self.zoneName, ownerName: CKCurrentUserDefaultName)
    }

    func accountStatus() async throws -> CloudAccountStatus {
        switch try await container.accountStatus() {
        case .available: .available
        case .noAccount: .noAccount
        default: .unavailable
        }
    }

    func ensureZone() async throws {
        let (saved, _) = try await database.modifyRecordZones(saving: [CKRecordZone(zoneID: zoneID)], deleting: [])
        for (_, result) in saved {
            _ = try result.get()
        }
    }

    func save(_ records: [CloudRecord]) async throws {
        var files: [URL] = []
        defer {
            for file in files {
                try? FileManager.default.removeItem(at: file)
            }
        }
        for start in stride(from: 0, to: records.count, by: Self.blockSize) {
            let block = try records[start..<min(start + Self.blockSize, records.count)].map { record in
                try ckRecord(record, files: &files)
            }
            do {
                // Overwrites without a change tag: every record has one writer.
                let (saved, _) = try await database.modifyRecords(
                    saving: block,
                    deleting: [],
                    savePolicy: .allKeys,
                    atomically: false
                )
                for (_, result) in saved {
                    if case .failure(let error) = result {
                        throw error
                    }
                }
            } catch let error as CKError where Self.zoneMissing(error) {
                throw CloudDatabaseError.zoneNotFound
            }
        }
    }

    func changes(since token: Data?) async throws -> CloudChanges {
        var serverToken = token.flatMap {
            try? NSKeyedUnarchiver.unarchivedObject(ofClass: CKServerChangeToken.self, from: $0)
        }
        var records: [CloudRecord] = []
        do {
            while true {
                let changes = try await database.recordZoneChanges(inZoneWith: zoneID, since: serverToken)
                // A record that did not come throws, so the token stays and
                // it comes with the next fetch.
                for (_, result) in changes.modificationResultsByID {
                    if let record = try Self.cloudRecord(result.get().record) {
                        records.append(record)
                    }
                }
                serverToken = changes.changeToken
                if !changes.moreComing { break }
            }
        } catch let error as CKError where error.code == .changeTokenExpired {
            throw CloudDatabaseError.tokenExpired
        } catch let error as CKError where Self.zoneMissing(error) {
            throw CloudDatabaseError.zoneNotFound
        }
        let data = serverToken.flatMap {
            try? NSKeyedArchiver.archivedData(withRootObject: $0, requiringSecureCoding: true)
        }
        return CloudChanges(records: records, token: data)
    }

    func deleteZone() async throws {
        do {
            // Errors of the zone itself come in the result, not thrown.
            let (_, deleted) = try await database.modifyRecordZones(saving: [], deleting: [zoneID])
            for (_, result) in deleted {
                try result.get()
            }
        } catch let error as CKError where Self.zoneMissing(error) {
            // Gone already.
        }
    }

    func subscribe() async throws {
        let subscription = CKRecordZoneSubscription(zoneID: zoneID, subscriptionID: Self.subscriptionId)
        let info = CKSubscription.NotificationInfo()
        info.shouldSendContentAvailable = true
        subscription.notificationInfo = info
        let (saved, _) = try await database.modifySubscriptions(saving: [subscription], deleting: [])
        for (_, result) in saved {
            _ = try result.get()
        }
    }

    private static func zoneMissing(_ error: CKError) -> Bool {
        error.code == .zoneNotFound || error.code == .userDeletedZone
    }

    // MARK: - Records

    private func ckRecord(_ record: CloudRecord, files: inout [URL]) throws -> CKRecord {
        let ck = CKRecord(
            recordType: record.kind.rawValue,
            recordID: CKRecord.ID(recordName: record.name, zoneID: zoneID)
        )
        ck["nasId"] = record.nasId
        ck["writtenAt"] = record.writtenAt
        ck["version"] = record.version
        ck["payload"] = record.payload
        switch record.kind {
        case .state:
            ck["observedAt"] = record.time
        case .day:
            ck["day"] = record.time
        }
        if let recent = record.recent {
            let file = FileManager.default.temporaryDirectory
                .appendingPathComponent("nasstats-recent-\(UUID().uuidString).json")
            try recent.write(to: file)
            files.append(file)
            ck["recent"] = CKAsset(fileURL: file)
        }
        return ck
    }

    /// The record of the transfer; `nil` for another type or missing fields.
    /// An asset that does not read throws.
    private static func cloudRecord(_ ck: CKRecord) throws -> CloudRecord? {
        guard let kind = CloudRecord.Kind(rawValue: ck.recordType),
              let nasId = ck["nasId"] as? String,
              let payload = ck["payload"] as? Data
        else {
            log.error("Record skipped: \(ck.recordID.recordName, privacy: .public)")
            return nil
        }
        let time: Int64? = switch kind {
        case .state: ck["observedAt"] as? Int64
        case .day: ck["day"] as? Int64
        }
        var recent: Data?
        if let asset = ck["recent"] as? CKAsset {
            guard let url = asset.fileURL else { throw CocoaError(.fileReadNoSuchFile) }
            recent = try Data(contentsOf: url)
        }
        return CloudRecord(
            kind: kind,
            nasId: nasId,
            time: time ?? 0,
            writtenAt: ck["writtenAt"] as? Int64 ?? 0,
            version: ck["version"] as? Int64 ?? 1,
            payload: payload,
            recent: recent
        )
    }
}
