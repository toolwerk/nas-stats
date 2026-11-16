import Foundation
import Testing

/// File names of the CSV export of the history screen (#45).
struct CSVExportTests {
    @Test func fileNameNamesKindNasAndRange() {
        #expect(
            CSVExport.fileName(kind: .history, nasName: "Keller", range: .month, demo: false)
                == "nas-stats-verlauf-keller-30-tage.csv")
        #expect(
            CSVExport.fileName(kind: .events, nasName: "Keller", range: .day, demo: false)
                == "nas-stats-ereignisse-keller-24h.csv")
        #expect(
            CSVExport.fileName(kind: .history, nasName: "Keller", range: .week, demo: false)
                == "nas-stats-verlauf-keller-7-tage.csv")
        #expect(
            CSVExport.fileName(kind: .history, nasName: "Keller", range: .year, demo: false)
                == "nas-stats-verlauf-keller-1-jahr.csv")
    }

    @Test func demoValuesAreMarkedInTheName() {
        #expect(
            CSVExport.fileName(kind: .history, nasName: "Keller", range: .month, demo: true)
                == "nas-stats-demo-verlauf-keller-30-tage.csv")
        #expect(
            CSVExport.fileName(kind: .events, nasName: "Büro", range: .week, demo: true)
                == "nas-stats-demo-ereignisse-buero-7-tage.csv")
    }

    @Test func slugIsLowerCaseAscii() {
        #expect(CSVExport.slug("Büro") == "buero")
        #expect(CSVExport.slug("ÄÖÜ ß") == "aeoeue-ss")
        #expect(CSVExport.slug("Café NAS") == "cafe-nas")
        #expect(CSVExport.slug("  QNAP / TS-464 (Keller)  ") == "qnap-ts-464-keller")
        #expect(CSVExport.slug("a--b__c") == "a-b-c")
        #expect(CSVExport.slug("🙂") == "nas")
        #expect(CSVExport.slug("") == "nas")
    }
}
