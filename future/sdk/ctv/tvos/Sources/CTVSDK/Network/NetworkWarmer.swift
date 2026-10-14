import Foundation

/// NetworkWarmer provides HTTP preconnect and DNS prefetch functionality
/// for tvOS to reduce cold-start latency for auction requests.
///
/// Key features:
/// - DNS prefetch on SDK init
/// - URLSession connection preconnect
/// - Connection pooling warmup
/// - Thread-safe singleton pattern
public final class NetworkWarmer: @unchecked Sendable {
    
    public static let shared = NetworkWarmer()
    
    // MARK: - State
    
    private let queue = DispatchQueue(label: "com.apexmediation.ctv.networkwarmer", qos: .utility)
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
        // tvOS typically runs on powerful hardware, use more connections
        config.httpMaximumConnectionsPerHost = 4
        config.timeoutIntervalForRequest = 5.0
        config.waitsForConnectivity = false
        config.urlCache = nil
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: config)
    }()
    
    private init() {}
    
    // MARK: - Public API
    
    /// Warm up connections to the specified endpoint.
    public func warmup(endpoint: String) {
        queue.async { [weak self] in
            guard let self = self else { return }
            guard !self.warmedEndpoints.contains(endpoint) else { return }
            
            guard let url = URL(string: endpoint), let host = url.host else { return }
            
            // DNS prefetch
            self.prefetchDns(hostname: host)
            
            // HTTP preconnect
            self.preconnect(endpoint: endpoint)
            
            self.warmedEndpoints.insert(endpoint)
            self._isWarmedUp = true
        }
    }
    
    /// Get cached DNS entries for a hostname.
    public func getCachedDns(hostname: String) -> [String]? {
        queue.sync { dnsCache[hostname] }
    }
    
    /// Get the shared URLSession with pre-warmed connections.
    public func getWarmedSession() -> URLSession {
        session
    }
    
    /// Reset state for testing.
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
                "cachedHosts": Array(dnsCache.keys)
            ]
        }
    }
    
    // MARK: - Private
    
    private func prefetchDns(hostname: String) {
        if dnsCache[hostname] != nil { return }
        
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
        
        let task = session.dataTask(with: request) { _, _, _ in }
        task.resume()
    }
}
