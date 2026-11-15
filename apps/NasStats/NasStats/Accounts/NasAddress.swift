import Foundation

/// Why an address typed by the user is not usable.
enum NasAddressError: Error, Equatable, Sendable {
    case empty
    /// `http://` instead of `https://`; the core only speaks HTTPS.
    case notHTTPS
    /// No host, a path, a query or a port outside 1…65535.
    case invalid
}

/// Checks and normalizes the address of a NAS.
enum NasAddress {
    /// Turns user input into `https://host[:port]` without a trailing slash.
    /// Input without a scheme gets `https://`.
    static func normalize(_ input: String) -> Result<String, NasAddressError> {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return .failure(.empty) }

        let lower = trimmed.lowercased()
        let withScheme: String
        if lower.hasPrefix("http://") {
            return .failure(.notHTTPS)
        } else if lower.hasPrefix("https://") {
            withScheme = trimmed
        } else if trimmed.contains("://") {
            return .failure(.invalid)
        } else {
            withScheme = "https://" + trimmed
        }

        guard let parts = URLComponents(string: withScheme),
              let host = parts.percentEncodedHost, !host.isEmpty,
              parts.path.isEmpty || parts.path == "/",
              parts.query == nil, parts.fragment == nil,
              parts.user == nil, parts.password == nil
        else { return .failure(.invalid) }
        if let port = parts.port, !(1...65_535).contains(port) {
            return .failure(.invalid)
        }

        var result = URLComponents()
        result.scheme = "https"
        result.percentEncodedHost = host
        result.port = parts.port
        guard let string = result.string else { return .failure(.invalid) }
        return .success(string)
    }

    /// Host of a normalized address, e.g. as a fallback name.
    static func host(of baseUrl: String) -> String? {
        guard let host = URLComponents(string: baseUrl)?.host, !host.isEmpty else {
            return nil
        }
        return host
    }
}
