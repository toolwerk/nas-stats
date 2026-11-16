import Foundation
import Network

/// Why a NAS without an answer is unreachable (#129). The core only knows
/// "no answer"; on the iPhone the usual causes are outside the app (not at
/// home, "Lokales Netzwerk" refused), so the detail page asks the system.
enum UnreachableReason: Equatable, Sendable, CaseIterable {
    case noNetwork
    case offHomeNetwork
    case localNetworkDenied
    /// The port accepts a connection, so the failing part is the query.
    case portReachable
    case noAnswer
}

/// Outcome of a TCP connection test to the port of the NAS.
enum ProbeConnection: Equatable, Sendable {
    case ready
    case localNetworkDenied
    case failed
    case timedOut
}

struct ProbeResult: Equatable, Sendable {
    /// The system has a network path at all.
    var pathSatisfied: Bool
    /// The path runs over Wi-Fi or cable.
    var onLocalInterface: Bool
    var connection: ProbeConnection
}

enum ReachabilityDiagnosis {
    /// Order of precedence: no path, refused local network, away from the
    /// home network, then what the connection said.
    static func classify(_ result: ProbeResult, hostIsLocal: Bool) -> UnreachableReason {
        if !result.pathSatisfied { return .noNetwork }
        if result.connection == .localNetworkDenied { return .localNetworkDenied }
        if hostIsLocal && !result.onLocalInterface { return .offHomeNetwork }
        if result.connection == .ready { return .portReachable }
        return .noAnswer
    }

    /// Whether a host name or address lies in the home network: private
    /// IPv4 and IPv6 ranges, link-local, `.local` and names without a dot.
    static func isLocalHost(_ host: String) -> Bool {
        var name = host.lowercased()
        if name.hasPrefix("["), name.hasSuffix("]") {
            name = String(name.dropFirst().dropLast())
        }
        if name.contains(":") {
            return isLocalIPv6(name)
        }
        let parts = name.split(separator: ".", omittingEmptySubsequences: false)
        if parts.count == 4, let octets = Optional(parts.compactMap { UInt8($0) }), octets.count == 4 {
            let (a, b) = (octets[0], octets[1])
            return a == 10 || (a == 172 && (16...31).contains(b))
                || (a == 192 && b == 168) || (a == 169 && b == 254)
        }
        return name.hasSuffix(".local") || !name.contains(".")
    }

    private static func isLocalIPv6(_ address: String) -> Bool {
        let bare = address.split(separator: "%", maxSplits: 1).first.map(String.init) ?? address
        guard let first = bare.split(separator: ":", omittingEmptySubsequences: false).first,
            let group = UInt16(first, radix: 16)
        else { return false }
        return (group & 0xfe00) == 0xfc00 || (group & 0xffc0) == 0xfe80
    }

    static func text(for reason: UnreachableReason) -> String {
        #if os(iOS)
        let device = "iPhone"
        let settings = "Einstellungen › Datenschutz & Sicherheit › Lokales Netzwerk"
        #else
        let device = "Mac"
        let settings = "Systemeinstellungen › Datenschutz & Sicherheit › Lokales Netzwerk"
        #endif
        switch reason {
        case .noNetwork:
            return "Das \(device) hat keine Netzwerkverbindung."
        case .offHomeNetwork:
            return "Das \(device) ist nicht im WLAN oder Heimnetz. Die Adresse des NAS gilt nur dort."
        case .localNetworkDenied:
            return "NAS Stats darf nicht ins lokale Netzwerk. Erlauben Sie es unter \(settings)."
        case .portReachable:
            return "Das NAS ist im Netz erreichbar, aber die Abfrage ist gescheitert. Prüfen Sie Anmeldung und Zertifikat in den Einstellungen des NAS."
        case .noAnswer:
            return "Das NAS antwortet nicht. Ist das \(device) im selben Netz wie das NAS? Prüfen Sie außerdem unter \(settings), ob NAS Stats zugreifen darf."
        }
    }

    /// Opens a TCP connection to the port of the NAS and sends nothing.
    /// `nil` without a host. Gives up after five seconds, which is also
    /// what happens while iOS still asks for the local network.
    static func probe(baseUrl: String) async -> ProbeResult? {
        guard let host = NasAddress.host(of: baseUrl),
            let components = URLComponents(string: baseUrl)
        else { return nil }
        let portNumber = UInt16(exactly: components.port ?? 443) ?? 443
        guard let port = NWEndpoint.Port(rawValue: portNumber) else { return nil }

        return await withCheckedContinuation { continuation in
            ProbeSession(host: host, port: port, continuation: continuation).start()
        }
    }
}

/// One connection test. All handlers run on `queue`, so the state needs no
/// lock; `finished` makes sure the continuation resumes exactly once.
private final class ProbeSession: @unchecked Sendable {
    private let queue = DispatchQueue(label: "net.staude.nasstats.probe")
    private let monitor = NWPathMonitor()
    private let connection: NWConnection
    private let continuation: CheckedContinuation<ProbeResult?, Never>
    private var path: NWPath?
    private var outcome: ProbeConnection?
    private var finished = false

    init(host: String, port: NWEndpoint.Port, continuation: CheckedContinuation<ProbeResult?, Never>) {
        connection = NWConnection(host: NWEndpoint.Host(host), port: port, using: .tcp)
        self.continuation = continuation
    }

    func start() {
        monitor.pathUpdateHandler = { [self] newPath in
            if path == nil {
                path = newPath
                finish()
            }
        }
        connection.stateUpdateHandler = { [self] state in
            switch state {
            case .ready:
                conclude(.ready)
            case .waiting:
                if connection.currentPath?.unsatisfiedReason == .localNetworkDenied {
                    conclude(.localNetworkDenied)
                }
            case .failed:
                conclude(
                    connection.currentPath?.unsatisfiedReason == .localNetworkDenied
                        ? .localNetworkDenied : .failed)
            default:
                break
            }
        }
        queue.asyncAfter(deadline: .now() + 5) { [self] in
            if outcome == nil { outcome = .timedOut }
            if path == nil { path = monitor.currentPath }
            finish()
        }
        monitor.start(queue: queue)
        connection.start(queue: queue)
    }

    private func conclude(_ value: ProbeConnection) {
        if outcome == nil { outcome = value }
        finish()
    }

    private func finish() {
        guard !finished, let path, let outcome else { return }
        finished = true
        monitor.cancel()
        connection.cancel()
        continuation.resume(returning: ProbeResult(
            pathSatisfied: path.status == .satisfied,
            onLocalInterface: path.usesInterfaceType(.wifi) || path.usesInterfaceType(.wiredEthernet),
            connection: outcome))
    }
}
