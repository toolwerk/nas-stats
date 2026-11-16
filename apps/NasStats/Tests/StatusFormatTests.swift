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

    @Test func flooredNeverReachesThreshold() {
        let t = FillThresholds.standard
        #expect(StatusFormat.floored(89.96, fractionDigits: 1) == 89.9)
        #expect(StatusFormat.floored(79.96, fractionDigits: 1) == 79.9)
        #expect(StatusFormat.floored(89.6, fractionDigits: 0) == 89)
        #expect(StatusFormat.floored(nil, fractionDigits: 0) == nil)
        for raw in [79.96, 89.96, 79.6, 89.6] {
            let shown = StatusFormat.floored(raw, fractionDigits: 1)
            #expect(t.level(for: shown) == t.level(for: raw))
        }
        #expect(StatusFormat.percent(StatusFormat.floored(89.96, fractionDigits: 1), locale: de) == "89,9 %")
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

    @Test func uptime() {
        #expect(StatusFormat.uptime(nil) == "n/a")
        #expect(StatusFormat.uptime(45 * 60) == "45 min")
        #expect(StatusFormat.uptime(30 * 3600) == "30 h")
        #expect(StatusFormat.uptime(47 * 3600 + 3599) == "47 h")
        #expect(StatusFormat.uptime(38 * 86400) == "38 Tage")
    }

    @Test func hours() {
        #expect(StatusFormat.hours(21_412, locale: de) == "21.412 h")
        #expect(StatusFormat.hours(312, locale: de) == "312 h")
        #expect(StatusFormat.hours(nil, locale: de) == "n/a")
    }

    @Test func runtime() {
        #expect(StatusFormat.runtime(34 * 60) == "34 min")
        #expect(StatusFormat.runtime(65 * 60) == "1 h 05 min")
        #expect(StatusFormat.runtime(130 * 60) == "2 h 10 min")
        #expect(StatusFormat.runtime(nil) == "n/a")
    }

    @Test func bytes() {
        #expect(StatusFormat.bytes(8_000_000_000, locale: de) == "7,5 GB")
        #expect(StatusFormat.bytes(8 * 1024 * 1024 * 1024, locale: de) == "8 GB")
        #expect(StatusFormat.bytes(512, locale: de) == "512 B")
        #expect(StatusFormat.bytes(1_073_700_000, locale: de) == "1 GB")
        #expect(StatusFormat.bytes(nil, locale: de) == "n/a")
    }

    @Test func throughput() {
        #expect(StatusFormat.throughput(rx: 4_200_000, tx: 300_000, locale: de) == "↓ 4,2 · ↑ 0,3 MB/s")
        #expect(StatusFormat.throughput(rx: nil, tx: 300_000, locale: de) == "↓ n/a · ↑ 0,3 MB/s")
        #expect(StatusFormat.throughput(rx: nil, tx: nil, locale: de) == "n/a")
    }

    @Test func nextRun() {
        // 2026-11-14 14:05 UTC
        let now = Date(timeIntervalSince1970: 1_794_665_100)
        func next(_ seconds: Double) -> String {
            StatusFormat.nextRun(now.addingTimeInterval(seconds), now: now, locale: de, timeZone: utc)
        }
        #expect(next(11 * 3600 + 55 * 60) == "morgen, 02:00")
        #expect(next(23 * 3600) == "morgen, 13:05")
        #expect(next(55 * 60) == "15:00")
        #expect(next(3 * 86400) == "17. Nov., 14:05")
        #expect(StatusFormat.nextRun(nil, now: now, locale: de, timeZone: utc) == "n/a")
    }

    @Test func observed() {
        let date = Date(timeIntervalSince1970: 1_794_665_100)
        #expect(StatusFormat.observed(date, now: date.addingTimeInterval(600), locale: de, timeZone: utc) == "Stand 14:05")
        #expect(
            StatusFormat.observed(date, now: date.addingTimeInterval(86400), locale: de, timeZone: utc)
                == "Stand 14. Nov., 14:05")
        #expect(
            StatusFormat.observed(date, now: date.addingTimeInterval(366 * 86400), locale: de, timeZone: utc)
                == "Stand 14. Nov. 2026, 14:05")
    }

    @Test func age() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        #expect(StatusFormat.age(now, now: now) == "gerade eben")
        #expect(StatusFormat.age(now.addingTimeInterval(-120), now: now) == "vor 2 min")
        #expect(StatusFormat.age(now.addingTimeInterval(-3 * 3600), now: now) == "vor 3 h")
        #expect(StatusFormat.age(now.addingTimeInterval(-50 * 3600), now: now) == "vor 2 Tagen")
    }
}
