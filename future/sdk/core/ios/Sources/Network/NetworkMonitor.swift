import Foundation
import Network

/// NetworkMonitor - Monitors network connectivity and provides fast-fail behavior.
///
/// This class provides:
/// - Real-time network state monitoring using NWPathMonitor
/// - Fast-fail for no-network conditions (no UI jank)
/// - Connection quality assessment
/// - Callback-based network state change notifications
@available(iOS 12.0, tvOS 12.0, macOS 10.14, *)
public final class NetworkMonitor {
    
    /// Shared instance for convenience
    public static let shared = NetworkMonitor()
    
    /// Fast-fail timeout when offline (seconds)
    public static let offlineFastFailTimeout: TimeInterval = 0.1
    
    /// Normal network timeout (seconds)
    public static let normalTimeout: TimeInterval = 10.0
    
    /// Network state information
    public struct NetworkState: Equatable {
        public let isConnected: Bool
        public let connectionType: ConnectionType
        public let isExpensive: Bool
        public let isConstrained: Bool
        public let timestamp: Date
        
        public static let offline = NetworkState(
            isConnected: false,
            connectionType: .none,
            isExpensive: false,
            isConstrained: false,
            timestamp: Date()
        )
    }
    
    /// Connection type enumeration
    public enum ConnectionType: String {
        case none
        case wifi
        case cellular
        case ethernet
        case loopback
        case other
    }
    
    /// Result of a pre-flight check before network operations
    public enum PreflightResult {
        /// Network is available, proceed with operation
        case proceed(NetworkState)
        
        /// Network unavailable, fast-fail with this error
        case fastFail(reason: String, state: NetworkState)
    }
    
    /// Listener closure type for network state changes
    public typealias NetworkStateListener = (NetworkState) -> Void
    
    private let monitor: NWPathMonitor
    private let queue = DispatchQueue(label: "com.rivalapex.networkmonitor", qos: .utility)
    
    private var currentState = NetworkState.offline {
        didSet {
            if oldValue != currentState {
                notifyListeners()
            }
        }
    }
    
    private var listeners: [UUID: NetworkStateListener] = [:]
    private let listenersLock = NSLock()
    
    private var isMonitoring = false
    
    public init() {
        self.monitor = NWPathMonitor()
        setupMonitor()
    }
    
    private func setupMonitor() {
        monitor.pathUpdateHandler = { [weak self] path in
            self?.updateState(from: path)
        }
    }
    
    /// Starts monitoring network state changes
    public func startMonitoring() {
        guard !isMonitoring else { return }
        isMonitoring = true
        monitor.start(queue: queue)
    }
    
    /// Stops monitoring network state changes
    public func stopMonitoring() {
        guard isMonitoring else { return }
        isMonitoring = false
        monitor.cancel()
    }
    
    /// Gets the current network state
    public var state: NetworkState {
        return currentState
    }
    
    /// Checks if network is currently connected
    public var isConnected: Bool {
        return currentState.isConnected
    }
    
    /// Checks if we're on an expensive connection (cellular data)
    public var isExpensive: Bool {
        return currentState.isExpensive
    }
    
    /// Performs a preflight check before network operations
    /// Returns immediately if offline, allowing fast-fail behavior
    public func preflight() -> PreflightResult {
        let state = currentState
        
        if state.isConnected {
            return .proceed(state)
        } else {
            return .fastFail(reason: "No network connection", state: state)
        }
    }
    
    /// Gets the appropriate timeout based on network state
    /// Returns fast-fail timeout when offline
    public func effectiveTimeout() -> TimeInterval {
        return isConnected ? Self.normalTimeout : Self.offlineFastFailTimeout
    }
    
    /// Adds a listener for network state changes
    /// - Returns: A token to use when removing the listener
    @discardableResult
    public func addListener(_ listener: @escaping NetworkStateListener) -> UUID {
        let token = UUID()
        listenersLock.lock()
        listeners[token] = listener
        listenersLock.unlock()
        
        // Immediately notify with current state
        listener(currentState)
        return token
    }
    
    /// Removes a listener using its token
    public func removeListener(token: UUID) {
        listenersLock.lock()
        listeners.removeValue(forKey: token)
        listenersLock.unlock()
    }
    
    /// Gets network quality hints for adaptive behavior
    public func qualityHints() -> [String: Any] {
        let state = currentState
        return [
            "isConnected": state.isConnected,
            "connectionType": state.connectionType.rawValue,
            "isExpensive": state.isExpensive,
            "isConstrained": state.isConstrained,
            "suggestedTimeout": effectiveTimeout(),
            "shouldReduceQuality": state.isExpensive || state.isConstrained
        ]
    }
    
    private func updateState(from path: NWPath) {
        let connectionType: ConnectionType
        if path.usesInterfaceType(.wifi) {
            connectionType = .wifi
        } else if path.usesInterfaceType(.cellular) {
            connectionType = .cellular
        } else if path.usesInterfaceType(.wiredEthernet) {
            connectionType = .ethernet
        } else if path.usesInterfaceType(.loopback) {
            connectionType = .loopback
        } else if path.status == .satisfied {
            connectionType = .other
        } else {
            connectionType = .none
        }
        
        currentState = NetworkState(
            isConnected: path.status == .satisfied,
            connectionType: connectionType,
            isExpensive: path.isExpensive,
            isConstrained: path.isConstrained,
            timestamp: Date()
        )
    }
    
    private func notifyListeners() {
        let state = currentState
        listenersLock.lock()
        let currentListeners = listeners
        listenersLock.unlock()
        
        for (_, listener) in currentListeners {
            DispatchQueue.main.async {
                listener(state)
            }
        }
    }
}

/// Error type for fast-fail on no-network conditions
public struct NoNetworkError: Error {
    public let message: String
    public let networkState: NetworkMonitor.NetworkState
    
    public init(message: String, networkState: NetworkMonitor.NetworkState) {
        self.message = message
        self.networkState = networkState
    }
}

/// Fallback for older iOS versions without NWPathMonitor
public final class LegacyNetworkMonitor {
    
    public static let shared = LegacyNetworkMonitor()
    
    /// Simple reachability check via URLSession
    public func checkConnectivity(timeout: TimeInterval = 2.0, completion: @escaping (Bool) -> Void) {
        guard let url = URL(string: "https://www.apple.com/library/test/success.html") else {
            completion(false)
            return
        }
        
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        request.timeoutInterval = timeout
        
        let task = URLSession.shared.dataTask(with: request) { _, response, _ in
            if let httpResponse = response as? HTTPURLResponse {
                completion(httpResponse.statusCode == 200)
            } else {
                completion(false)
            }
        }
        task.resume()
    }
    
    /// Synchronous connectivity check (use with caution - blocks thread)
    public func isConnectedSync(timeout: TimeInterval = 0.5) -> Bool {
        let semaphore = DispatchSemaphore(value: 0)
        var connected = false
        
        checkConnectivity(timeout: timeout) { result in
            connected = result
            semaphore.signal()
        }
        
        _ = semaphore.wait(timeout: .now() + timeout + 0.1)
        return connected
    }
}
