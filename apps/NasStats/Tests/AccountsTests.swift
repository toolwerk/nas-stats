import Foundation
import Testing

/// Address check, error texts and account data of the add dialog (#14).
/// No keychain: the tests run without signing.
struct AccountsTests {
    @Test(arguments: [
        ("192.168.1.5:443", "https://192.168.1.5:443"),
        ("https://nas.local/", "https://nas.local"),
        ("  https://nas.local:8081  ", "https://nas.local:8081"),
        ("HTTPS://nas.local", "https://nas.local"),
        ("nas.local", "https://nas.local"),
    ])
    func validAddressesAreNormalized(input: String, expected: String) {
        #expect(NasAddress.normalize(input) == .success(expected))
    }

    @Test(arguments: [
        ("", NasAddressError.empty),
        ("   ", .empty),
        ("http://x", .notHTTPS),
        ("HTTP://nas.local:8080", .notHTTPS),
        ("https://nas.local:99999", .invalid),
        ("https://nas.local:0", .invalid),
        ("https://nas.local/foo", .invalid),
        ("https://nas.local/?a=1", .invalid),
        ("ftp://nas.local", .invalid),
        ("https://", .invalid),
    ])
    func invalidAddressesAreRefused(input: String, expected: NasAddressError) {
        #expect(NasAddress.normalize(input) == .failure(expected))
    }

    @Test func hostOfAnAddress() {
        #expect(NasAddress.host(of: "https://192.168.1.5:443") == "192.168.1.5")
        #expect(NasAddress.host(of: "https://nas.local") == "nas.local")
    }

    @Test func eachLoginProblemHasItsOwnText() {
        let errors: [FfiError] = [
            .LoginFailed,
            .SecondFactorRequired,
            .PermissionDenied,
            .Unreachable(detail: "refused"),
        ]
        let texts = errors.map(connectionErrorText)
        #expect(texts.allSatisfy { !$0.isEmpty })
        #expect(Set(texts).count == errors.count)
        #expect(connectionErrorText(FfiError.PermissionDenied).contains("System Monitoring"))
        #expect(connectionErrorText(FfiError.Timeout) == connectionErrorText(FfiError.Unreachable(detail: "x")))
        #expect(connectionErrorText(FfiError.Http(status: 503)).contains("503"))
        #expect(connectionErrorText(FfiError.InvalidInput(detail: "kaputt")).contains("kaputt"))
    }

    @Test func addressErrorsHaveTexts() {
        let texts = [NasAddressError.empty, .notHTTPS, .invalid].map(connectionErrorText)
        #expect(Set(texts).count == 3)
        #expect(texts.allSatisfy { !$0.isEmpty })
    }

    @Test func storedAccountSurvivesJSON() throws {
        let stored = StoredAccount(account: account(), password: "geheim")
        let data = try JSONEncoder().encode(stored)
        let decoded = try JSONDecoder().decode(StoredAccount.self, from: data)
        #expect(decoded == stored)
    }

    @Test func storedAccountWithoutModelSurvivesJSON() throws {
        var account = account()
        account.model = nil
        let stored = StoredAccount(account: account, password: "geheim")
        let decoded = try JSONDecoder().decode(
            StoredAccount.self, from: JSONEncoder().encode(stored)
        )
        #expect(decoded.account.model == nil)
    }

    @Test func qnapAccountGivesACoreConfig() throws {
        let account = account()
        let config = try #require(account.config)
        #expect(config.id == account.id.uuidString)
        #expect(config.provider == .qnap)
        #expect(config.baseUrl == account.baseUrl)
        #expect(config.fingerprint == account.fingerprint)
    }

    @Test func unknownProviderGivesNoConfig() {
        var account = account()
        account.providerKey = "synology-v9"
        #expect(account.config == nil)
    }

    /// The config is accepted by the core as it is; nothing is sent.
    @Test func coreAcceptsTheConfig() throws {
        let account = account()
        let config = try #require(account.config)
        let core = NasCore()
        try core.addNas(config: config, user: account.user, password: "demo")
        #expect(core.states().map(\.nasId) == [account.id.uuidString])
    }

    private func account() -> NasAccount {
        NasAccount(
            id: UUID(),
            name: "Keller",
            providerKey: NasAccount.qnapKey,
            baseUrl: "https://nas.demo.invalid:443",
            fingerprint: String(repeating: "AB:", count: 31) + "AB",
            user: "monitor",
            model: "TS-464",
            syncsViaICloud: true
        )
    }
}
