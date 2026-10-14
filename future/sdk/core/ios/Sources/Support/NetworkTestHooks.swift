import Foundation

/// Allows tests to inject custom `URLProtocol` subclasses so networking code can be
/// executed deterministically without real network access.
public enum NetworkTestHooks {
    /// Register a mock protocol that should intercept outgoing URLSession requests.
    public static func registerMockProtocol(_ protocolClass: URLProtocol.Type) {
        URLProtocolRegistry.shared.register(protocolClass)
    }
    
    /// Clear all registered mock protocols.
    public static func resetMockProtocols() {
        URLProtocolRegistry.shared.reset()
    }

    /// Returns true when any mock URLProtocol classes are currently registered.
    public static var hasMockProtocols: Bool {
        URLProtocolRegistry.shared.hasProtocols()
    }
}

private final class URLProtocolRegistry: @unchecked Sendable {
    static let shared = URLProtocolRegistry()
    
    private var protocolClasses: [URLProtocol.Type] = []
    private let lock = NSLock()
    
    func register(_ protocolClass: URLProtocol.Type) {
        lock.lock()
        if !protocolClasses.contains(where: { $0 == protocolClass }) {
            protocolClasses.append(protocolClass)
        }
        lock.unlock()
    }
    
    func reset() {
        lock.lock()
        protocolClasses.removeAll()
        lock.unlock()
    }
    
    func hasProtocols() -> Bool {
        lock.lock()
        let hasAny = !protocolClasses.isEmpty
        lock.unlock()
        return hasAny
    }
    
    func apply(to configuration: URLSessionConfiguration) {
        lock.lock()
        let classes = protocolClasses
        lock.unlock()
        guard !classes.isEmpty else { return }
        configuration.protocolClasses = classes + (configuration.protocolClasses ?? [])
    }
}

extension URLSessionConfiguration {
    /// Returns the default configuration augmented with any registered mock protocols and optional timeouts.
    static func apexDefault(
        requestTimeout: TimeInterval? = nil,
        resourceTimeout: TimeInterval? = nil
    ) -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.default
        if let requestTimeout {
            configuration.timeoutIntervalForRequest = requestTimeout
        }
        if let resourceTimeout {
            configuration.timeoutIntervalForResource = resourceTimeout
        }
        URLProtocolRegistry.shared.apply(to: configuration)
        return configuration
    }
}
