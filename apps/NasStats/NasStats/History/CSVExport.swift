import CoreTransferable
import Foundation
import UniformTypeIdentifiers

/// A CSV file of the history screen (#45), made only when it is shared or
/// saved, so the screen does not build it on every load.
struct CSVExport: Transferable, Sendable {
    /// What the file holds.
    enum Kind: Sendable {
        case history
        case events

        fileprivate var slug: String {
            switch self {
            case .history: "verlauf"
            case .events: "ereignisse"
            }
        }
    }

    let fileName: String
    let make: @MainActor @Sendable () throws -> String

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .commaSeparatedText) { export in
            let text = try await export.make()
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(export.fileName)
            try Data(text.utf8).write(to: url, options: .atomic)
            return SentTransferredFile(url)
        }
    }

    /// `nas-stats[-demo]-<verlauf|ereignisse>-<nas>-<range>.csv`; the prefix
    /// `-demo` marks demo values.
    static func fileName(kind: Kind, nasName: String, range: HistoryRange, demo: Bool) -> String {
        let prefix = demo ? "nas-stats-demo" : "nas-stats"
        return "\(prefix)-\(kind.slug)-\(slug(nasName))-\(range.fileSlug).csv"
    }

    /// Lower case ASCII for a file name: umlauts as `ae`, `oe`, `ue`, `ß` as
    /// `ss`, other accents dropped, every other character a single `-`,
    /// none at the ends; `nas` when nothing is left.
    static func slug(_ name: String) -> String {
        var text = name.lowercased()
        for (umlaut, ascii) in [("ä", "ae"), ("ö", "oe"), ("ü", "ue"), ("ß", "ss")] {
            text = text.replacingOccurrences(of: umlaut, with: ascii)
        }
        text = text.folding(options: .diacriticInsensitive, locale: nil)
        var out = ""
        for char in text.unicodeScalars {
            if ("a"..."z").contains(char) || ("0"..."9").contains(char) {
                out.unicodeScalars.append(char)
            } else if !out.isEmpty, !out.hasSuffix("-") {
                out.append("-")
            }
        }
        while out.hasSuffix("-") { out.removeLast() }
        return out.isEmpty ? "nas" : out
    }
}

extension HistoryRange {
    /// Part of the CSV file name.
    var fileSlug: String {
        switch self {
        case .day: "24h"
        case .week: "7-tage"
        case .month: "30-tage"
        case .year: "1-jahr"
        }
    }
}
