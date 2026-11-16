import Foundation
import Testing

/// Poster double: records every notification and every withdrawal.
@MainActor
final class FakePoster: NotificationPoster {
    struct Post: Equatable {
        var id: String
        var group: String
        var title: String
        var body: String
        var delay: TimeInterval?
    }

    private(set) var posts: [Post] = []
    private(set) var withdrawn: [String] = []
    private(set) var authorizationRequests = 0
    var denied = false
    var authorized = true
    /// Delay of every `reschedulePending` call.
    private(set) var rescheduled: [TimeInterval?] = []

    func post(id: String, group: String, title: String, body: String, delay: TimeInterval?) {
        posts.append(Post(id: id, group: group, title: title, body: body, delay: delay))
    }

    func withdraw(ids: [String]) {
        withdrawn += ids
    }

    func requestAuthorization() async {
        authorizationRequests += 1
    }

    func isDenied() async -> Bool { denied }

    func isAuthorized() async -> Bool { authorized }

    func reschedulePending(delay: TimeInterval?) {
        rescheduled.append(delay)
    }
}

@MainActor
struct EventNotifierTests {
    let defaults: UserDefaults
    let poster = FakePoster()
    static let nas = "nas-1"

    init() throws {
        defaults = try #require(UserDefaults(suiteName: "nasstats-notifier-\(UUID().uuidString)"))
    }

    static func event(
        _ kind: EventKind = .diskTemperature,
        subject: String? = "3",
        level: Health = .warning,
        text: String = "Platte 3 bei 52 °C",
        startedAt: Int64 = 1_000
    ) -> NasEvent {
        NasEvent(
            nasId: nas, kind: kind, subject: subject, level: level,
            text: text, startedAt: startedAt, endedAt: nil
        )
    }

    func notifier(at date: Date = Date(timeIntervalSince1970: 0)) async -> EventNotifier {
        let notifier = EventNotifier(poster: poster, defaults: defaults, clock: { date }, calendar: Self.berlin)
        await notifier.refreshAuthorization()
        return notifier
    }

    static var berlin: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "Europe/Berlin")!
        return calendar
    }

    /// 16 Nov 2026 at `hour:minute` in Berlin.
    static func berlin(_ hour: Int, _ minute: Int = 0) -> Date {
        berlin.date(from: DateComponents(year: 2026, month: 11, day: 16, hour: hour, minute: minute))!
    }

    @Test func anOpenEventNotifiesOnceAndAgainOnlyAfterARecovery() async {
        let notifier = await notifier()
        let first = Self.event()
        notifier.update(nasId: Self.nas, name: "Keller", open: [first], settings: .init())
        #expect(poster.posts == [FakePoster.Post(
            id: EventNotifier.id(of: first), group: Self.nas,
            title: "Keller", body: "Platte 3 bei 52 °C", delay: nil
        )])

        notifier.update(nasId: Self.nas, name: "Keller", open: [first], settings: .init())
        #expect(poster.posts.count == 1)

        notifier.update(nasId: Self.nas, name: "Keller", open: [], settings: .init())
        let again = Self.event(startedAt: 2_000)
        notifier.update(nasId: Self.nas, name: "Keller", open: [again], settings: .init())
        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: first), EventNotifier.id(of: again)])
    }

    @Test func aRiseToCriticalNotifiesAgainWithTheSameId() async {
        let notifier = await notifier()
        let warning = Self.event(.volumeFill, subject: "vol1", text: "Volume 1 zu 82 % voll")
        notifier.update(nasId: Self.nas, name: "Keller", open: [warning], settings: .init())
        var critical = warning
        critical.level = .critical
        critical.text = "Volume 1 zu 91 % voll"
        notifier.update(nasId: Self.nas, name: "Keller", open: [critical], settings: .init())
        notifier.update(nasId: Self.nas, name: "Keller", open: [critical], settings: .init())

        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: warning), EventNotifier.id(of: warning)])
        #expect(poster.posts.last?.body == "Volume 1 zu 91 % voll")
    }

    @Test func anEndedEventWithdrawsItsNotification() async {
        let notifier = await notifier()
        let event = Self.event()
        notifier.update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        notifier.update(nasId: Self.nas, name: "Keller", open: [], settings: .init())
        #expect(poster.withdrawn == [EventNotifier.id(of: event)])
        // Nothing is left to withdraw on the next query.
        notifier.update(nasId: Self.nas, name: "Keller", open: [], settings: .init())
        #expect(poster.withdrawn.count == 1)
    }

    @Test func aKindSwitchedOffStaysSilentAlsoAfterSwitchingItOn() async {
        let notifier = await notifier()
        let event = Self.event()
        var off = NotificationSettings()
        off.disabled = [.disks]
        notifier.update(nasId: Self.nas, name: "Keller", open: [event], settings: off)
        notifier.update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        #expect(poster.posts.isEmpty)

        // Other kinds still notify.
        let ups = Self.event(.upsOnBattery, subject: nil, text: "USV im Akkubetrieb")
        notifier.update(nasId: Self.nas, name: "Keller", open: [event, ups], settings: off)
        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: ups)])
    }

    @Test func inTheQuietHoursTheNotificationWaitsForTheirEnd() async {
        let notifier = await notifier(at: Self.berlin(23))
        var settings = NotificationSettings()
        settings.quietHours.enabled = true
        notifier.update(nasId: Self.nas, name: "Keller", open: [Self.event()], settings: settings)
        #expect(poster.posts.first?.delay == 8 * 3600.0)
    }

    @Test func switchingAKindOffWithdrawsItsNotifications() async {
        let notifier = await notifier()
        let disk = Self.event()
        let ups = Self.event(.upsOnBattery, subject: nil, text: "USV im Akkubetrieb")
        notifier.update(nasId: Self.nas, name: "Keller", open: [disk, ups], settings: .init())
        var off = NotificationSettings()
        off.disabled = [.disks]
        notifier.apply(off, previous: .init())
        #expect(poster.withdrawn == [EventNotifier.id(of: disk)])

        // Switched on again, the event stays silent and is not withdrawn twice.
        notifier.apply(.init(), previous: off)
        notifier.update(nasId: Self.nas, name: "Keller", open: [disk, ups], settings: .init())
        #expect(poster.posts.count == 2)
        #expect(poster.withdrawn.count == 1)
        #expect(poster.rescheduled.isEmpty)
    }

    @Test func changedQuietHoursMovePendingNotifications() async {
        let notifier = await notifier(at: Self.berlin(23))
        var quiet = NotificationSettings()
        quiet.quietHours.enabled = true
        var later = quiet
        later.quietHours.endMinute = 8 * 60
        notifier.apply(later, previous: quiet)
        notifier.apply(.init(), previous: later)
        #expect(poster.rescheduled == [9 * 3600.0, nil])
    }

    @Test func withoutPermissionAnEventWaitsUntilItIsGiven() async {
        poster.authorized = false
        let notifier = await notifier()
        let event = Self.event()
        notifier.update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        #expect(poster.posts.isEmpty)

        poster.authorized = true
        await notifier.refreshAuthorization()
        notifier.update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        #expect(poster.posts.map(\.id) == [EventNotifier.id(of: event)])
    }

    @Test func aRestartDoesNotNotifyALastingEventAgain() async {
        let event = Self.event()
        await notifier().update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        let restarted = await notifier()
        restarted.update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        #expect(poster.posts.count == 1)
        // The restarted notifier still knows what to withdraw.
        restarted.update(nasId: Self.nas, name: "Keller", open: [], settings: .init())
        #expect(poster.withdrawn == [EventNotifier.id(of: event)])
    }

    @Test func forgetWithdrawsEveryNotificationOfTheNas() async {
        let notifier = await notifier()
        let disk = Self.event()
        let ups = Self.event(.upsOnBattery, subject: nil, text: "USV im Akkubetrieb")
        notifier.update(nasId: Self.nas, name: "Keller", open: [disk, ups], settings: .init())
        notifier.forget(nasId: Self.nas)
        #expect(Set(poster.withdrawn) == [EventNotifier.id(of: disk), EventNotifier.id(of: ups)])
        // Forgotten: the same events notify again.
        notifier.update(nasId: Self.nas, name: "Keller", open: [disk], settings: .init())
        #expect(poster.posts.count == 3)
    }

    @Test func otherNasKeepTheirNotifications() async {
        let notifier = await notifier()
        let event = Self.event()
        notifier.update(nasId: Self.nas, name: "Keller", open: [event], settings: .init())
        notifier.update(nasId: "nas-2", name: "Büro", open: [], settings: .init())
        notifier.forget(nasId: "nas-2")
        #expect(poster.withdrawn.isEmpty)
    }

    // MARK: - Quiet hours

    @Test func quietHoursOverMidnight() {
        let hours = QuietHours(enabled: true, startMinute: 22 * 60, endMinute: 7 * 60)
        #expect(hours.delay(at: Self.berlin(23), calendar: Self.berlin) == 8 * 3600.0)
        #expect(hours.delay(at: Self.berlin(6, 30), calendar: Self.berlin) == 30 * 60.0)
        #expect(hours.delay(at: Self.berlin(12), calendar: Self.berlin) == nil)
        #expect(hours.delay(at: Self.berlin(7), calendar: Self.berlin) == nil)
    }

    @Test func quietHoursWithinADay() {
        let hours = QuietHours(enabled: true, startMinute: 9 * 60, endMinute: 17 * 60)
        #expect(hours.delay(at: Self.berlin(10), calendar: Self.berlin) == 7 * 3600.0)
        #expect(hours.delay(at: Self.berlin(18), calendar: Self.berlin) == nil)
    }

    @Test func quietHoursWithTheSameStartAndEndHoldNothing() {
        let hours = QuietHours(enabled: true, startMinute: 600, endMinute: 600)
        #expect(hours.delay(at: Self.berlin(10), calendar: Self.berlin) == nil)
    }

    @Test func settingsRoundTripThroughTheDefaults() {
        var settings = NotificationSettings()
        settings.disabled = [.fans, .backups]
        settings.quietHours = QuietHours(enabled: true, startMinute: 1380, endMinute: 360)
        settings.save(to: defaults)
        #expect(NotificationSettings.load(from: defaults) == settings)
    }
}
