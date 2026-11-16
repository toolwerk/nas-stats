import Testing

/// Reason a NAS without an answer is unreachable, and which hosts count as
/// home network (#129). No network: the probe itself is not tested.
struct ReachabilityDiagnosisTests {
    private func result(
        path: Bool = true, local: Bool = true, _ connection: ProbeConnection
    ) -> ProbeResult {
        ProbeResult(pathSatisfied: path, onLocalInterface: local, connection: connection)
    }

    @Test func eachCauseGetsItsReason() {
        #expect(ReachabilityDiagnosis.classify(result(path: false, .failed), hostIsLocal: true) == .noNetwork)
        #expect(ReachabilityDiagnosis.classify(result(.localNetworkDenied), hostIsLocal: true) == .localNetworkDenied)
        #expect(ReachabilityDiagnosis.classify(result(local: false, .failed), hostIsLocal: true) == .offHomeNetwork)
        #expect(ReachabilityDiagnosis.classify(result(.ready), hostIsLocal: true) == .portReachable)
        #expect(ReachabilityDiagnosis.classify(result(.timedOut), hostIsLocal: true) == .noAnswer)
        #expect(ReachabilityDiagnosis.classify(result(.failed), hostIsLocal: true) == .noAnswer)
    }

    @Test func precedence() {
        // No path beats a refused local network.
        #expect(ReachabilityDiagnosis.classify(result(path: false, .localNetworkDenied), hostIsLocal: true) == .noNetwork)
        // A refused local network beats "not at home".
        #expect(ReachabilityDiagnosis.classify(result(local: false, .localNetworkDenied), hostIsLocal: true) == .localNetworkDenied)
        // A public host is reachable over mobile data.
        #expect(ReachabilityDiagnosis.classify(result(local: false, .ready), hostIsLocal: false) == .portReachable)
        #expect(ReachabilityDiagnosis.classify(result(local: false, .timedOut), hostIsLocal: false) == .noAnswer)
        // A local host with a local interface and a ready port is not "off home".
        #expect(ReachabilityDiagnosis.classify(result(.ready), hostIsLocal: true) == .portReachable)
    }

    @Test func reasonsHaveDistinctTexts() {
        let texts = UnreachableReason.allCases.map { ReachabilityDiagnosis.text(for: $0) }
        #expect(Set(texts).count == UnreachableReason.allCases.count)
    }

    @Test(arguments: [
        "10.0.0.5", "192.168.1.20", "172.16.0.1", "172.31.255.1", "169.254.1.1",
        "nas.local", "NAS.LOCAL", "nas", "fd12:3456::1", "fc00::1", "fe80::1",
        "[fe80::1]", "fe80::1%en0", "[fd00::5]",
    ])
    func homeHosts(host: String) {
        #expect(ReachabilityDiagnosis.isLocalHost(host))
    }

    @Test(arguments: [
        "8.8.8.8", "172.32.0.1", "172.15.0.1", "192.169.0.1", "nas.example.com",
        "2001:db8::1", "[2a00:1450::1]", "fec0::1",
    ])
    func otherHosts(host: String) {
        #expect(!ReachabilityDiagnosis.isLocalHost(host))
    }
}
