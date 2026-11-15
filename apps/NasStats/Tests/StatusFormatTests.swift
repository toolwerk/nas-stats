import Foundation
import Testing

/// Texts and thresholds of the status building blocks (#16).
struct StatusFormatTests {
    private let de = Locale(identifier: "de_DE")
    private let utc = TimeZone(identifier: "UTC")!

    @Test func thresholds() {
        let t = FillThresholds.standard
        #expect(t.level(for: 79.9) == .ok)
        #expect(t.level(for: 80) == .warning)
        #expect(t.level(for: 89.9) == .warning)
        #expect(t.level(for: 90) == .critical)
        #expect(t.level(for: nil) == .unknown)
    }

    @Test func percent() {
        #expect(StatusFormat.percent(87.1, locale: de) == "87,1 %")
        #expect(StatusFormat.percent(64, fractionDigits: 0, locale: de) == "64 %")
        #expect(StatusFormat.percent(nil, locale: de) == "n/a")
        #expect(StatusFormat.percent(104, fractionDigits: 0, locale: de) == "104 %")
    }

    @Test func capacity() {
        #expect(
            StatusFormat.capacity(used: 20_904_000_000_000, total: 24_000_000_000_000, locale: de)
                == "19,0 von 21,8 TB")
        #expect(StatusFormat.capacity(used: 512, total: 2048, locale: de) == "0,5 von 2,0 KB")
        #expect(StatusFormat.capacity(used: nil, total: 2048, locale: de) == nil)
        #expect(StatusFormat.capacity(used: 1, total: nil, locale: de) == nil)
    }

    @Test func temperature() {
        #expect(StatusFormat.temperature(52, locale: de) == "52 °C")
        #expect(StatusFormat.temperature(nil, locale: de) == "n/a")
    }

    @Test func forecast() {
        func main(_ days: Double) -> String {
            StatusFormat.forecast(.full(days: days, date: nil), locale: de, timeZone: utc).main
        }
        #expect(StatusFormat.forecast(.noTrend, locale: de, timeZone: utc).main == "kein Trend")
        #expect(main(0.4) == "heute voll")
        #expect(main(1) == "voll in 1 Tag")
        #expect(main(143) == "voll in 143 Tagen")
        #expect(main(364.9) == "voll in 364 Tagen")
        #expect(main(657) == "voll in 1,8 Jahren")

        // 2027-04-06 12:00 UTC
        let date = Date(timeIntervalSince1970: 1_807_012_800)
        let withDate = StatusFormat.forecast(.full(days: 143, date: date), locale: de, timeZone: utc)
        #expect(withDate.date == "etwa 6. April 2027")
    }

    @Test func backupAge() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        func age(_ seconds: Double) -> String {
            StatusFormat.backupAge(now.addingTimeInterval(-seconds), now: now)
        }
        #expect(StatusFormat.backupAge(nil, now: now) == "n/a")
        #expect(age(30) == "gerade eben")
        #expect(age(-300) == "gerade eben")
        #expect(age(5 * 60) == "vor 5 min")
        #expect(age(9 * 3600) == "vor 9 h")
        #expect(age(47 * 3600) == "vor 47 h")
        #expect(age(72 * 3600) == "vor 3 Tagen")
    }
}
