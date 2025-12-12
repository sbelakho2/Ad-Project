import Foundation
#if canImport(Network)
import Network
#endif

/// NetworkWarmer provides HTTP preconnect and DNS prefetch functionality
/// to reduce cold-start latency for auction requests.
///
/// Key features:
/// - DNS prefetch on SDK init (caches DNS resolution)
/// - URLSession connection preconnect (establishes TLS early)
/// - Connection pooling warmup
/// - Thread-safe singleton pattern
///
/// Usage:
/// ```swift
/// // At SDK init time:
/// NetworkWarmer.shared.warmup(endpoint: "https://api.apexmediation.ee")
///
/// // Check status:
/// if NetworkWarmer.shared.isWarmedUp { ... }
/// ```
public final class NetworkWarmer: @unchecked Sendable {
    
    public static let shared = NetworkWarmer()
    
    // MARK: - State
    
    private let queue = DispatchQueue(label: "com.apexmediation.networkwarmer", qos: .utility)
    private var dnsCache: [String: [String]] = [:]
    private var warmedEndpoints: Set<String> = []
    private var _isWarmedUp = false
    
    /// Whether at least one endpoint has been warmed up
    public var isWarmedUp: Bool {
        queue.sync { _isWarmedUp }
    }
    
    // Shared URLSession with optimal connection configuration
    private lazy var session: URLSession = {
        let config = URLSessionConfiguration.default
        config.httpMaximumConnectionsPerHost = ProcessInfo.processInfo.activeProcessorCount.clamped(to: 2...8)
        // Use short timeout for requests (connection timeout is controlled by the OS)
        config.timeoutIntervalForRequest = 5.0
        config.waitsForConnectivity = false
        config.urlCache = nil // We handle caching separately
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        
        // Enable HTTP/2
        if #available(iOS 13.0, tvOS 13.0, *) {
            config.allowsConstrainedNetworkAccess = true
            config.allowsExpensiveNetworkAccess = true
        }
        
        return URLSession(configuration: config)
    }()
    
    private init() {}
    
    // MARK: - Public API
    
    /// Warm up connections to the specified endpoint.
    /// This performs:
    /// 1. DNS prefetch (resolves hostname to IP addresses)
    /// 2. HTTP preconnect (establishes TCP + TLS connection)
    ///
    /// Call this at SDK initialization time for best cold-start performance.
    ///
    /// - Parameter endpoint: Base URL of the auction endpoint (e.g., "https://api.apexmediation.ee")
    public func warmup(endpoint: String) {
        queue.async { [weak self] in
            guard let self = self else { return }
            guard !self.warmedEndpoints.contains(endpoint) else { return }
            
            guard let url = URL(string: endpoint), let host = url.host else { return }
            
            // Step 1: DNS prefetch
            self.prefetchDns(hostname: host)
            
            // Step 2: HTTP preconnect
            self.preconnect(endpoint: endpoint)
            
            self.warmedEndpoints.insert(endpoint)
            self._isWarmedUp = true
        }
    }
    
    /// Get cached DNS entries for a hostname, if available.
    public func getCachedDns(hostname: String) -> [String]? {
        queue.sync { dnsCache[hostname] }
    }
    
    /// Get the shared URLSession with pre-warmed connections.
    public func getWarmedSession() -> URLSession {
        session
    }
    
    /// Clear all cached DNS entries and warmed endpoints.
    /// Primarily for testing.
    public func reset() {
        queue.sync {
            dnsCache.removeAll()
            warmedEndpoints.removeAll()
            _isWarmedUp = false
        }
    }
    
    /// Get diagnostics about current warmup state.
    public func getDiagnostics() -> [String: Any] {
        queue.sync {
            [
                "isWarmedUp": _isWarmedUp,
                "warmedEndpoints": Array(warmedEndpoints),
                "dnsCacheSize": dnsCache.count,
                "cachedHosts": Array(dnsCache.keys),
                "connectionPoolSize": ProcessInfo.processInfo.activeProcessorCount.clamped(to: 2...8)
            ]
        }
    }
    
    // MARK: - Private Implementation
    
    private func prefetchDns(hostname: String) {
        // Check cache first
        if dnsCache[hostname] != nil { return }
        
        // Use CFHost for DNS resolution
        let hostRef = CFHostCreateWithName(nil, hostname as CFString).takeRetainedValue()
        var resolved = DarwinBoolean(false)
        
        CFHostStartInfoResolution(hostRef, .addresses, nil)
        
        if let addresses = CFHostGetAddressing(hostRef, &resolved)?.takeUnretainedValue() as? [Data] {
            var ipStrings: [String] = []
            
            for addressData in addresses {
                addressData.withUnsafeBytes { ptr in
                    guard let sockaddr = ptr.baseAddress?.assumingMemoryBound(to: sockaddr.self) else { return }
                    
                    var hostname = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                    if getnameinfo(sockaddr, socklen_t(addressData.count),
                                   &hostname, socklen_t(hostname.count),
                                   nil, 0, NI_NUMERICHOST) == 0 {
                        ipStrings.append(String(cString: hostname))
                    }
                }
            }
            
            if !ipStrings.isEmpty {
                dnsCache[hostname] = ipStrings
            }
        }
    }
    
    private func preconnect(endpoint: String) {
        guard let url = URL(string: endpoint + "/health") else { return }
        
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.timeoutInterval = 2.0
        
        // Fire-and-forget HEAD request to warm connection
        let task = session.dataTask(with: request) { _, _, _ in
            // Preconnect is best-effort; ignore result
        }
        task.resume()
    }
}

// MARK: - Comparable Clamping Extension

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
