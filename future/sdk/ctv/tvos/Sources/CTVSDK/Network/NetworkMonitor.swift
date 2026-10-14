import Foundation
import Network

/// NetworkMonitor for tvOS - Monitors network connectivity for CTV applications.
///
/// This class provides:
/// - Real-time network state monitoring using NWPathMonitor
/// - Fast-fail for no-network conditions (critical for CTV ad breaks)
/// - Connection quality assessment optimized for streaming
/// - Callback-based network state change notifications
@available(tvOS 12.0, macOS 10.14, *)
public final class NetworkMonitor {
    
    /// Shared instance for convenience
    public static let shared = NetworkMonitor()
    
    /// Fast-fail timeout when offline (seconds) - shorter for CTV ad breaks
    public static let offlineFastFailTimeout: TimeInterval = 0.05
    
    /// Normal network timeout (seconds)
    public static let normalTimeout: TimeInterval = 8.0
    
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
    
    /// Connection type enumeration - tvOS typically uses ethernet or wifi
    public enum ConnectionType: String {
        case none
        case wifi
        case ethernet
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
    private let queue = DispatchQueue(label: "com.rivalapex.ctv.networkmonitor", qos: .utility)
    
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
    
    /// Last connectivity change time for rate limiting reconnection attempts
    private var lastConnectivityChange: Date?
    
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
    
    /// Performs a preflight check before network operations
    /// Returns immediately if offline, allowing fast-fail behavior
    public func preflight() -> PreflightResult {
        let state = currentState
        
        if state.isConnected {
            return .proceed(state)
        } else {
            return .fastFail(reason: "No network connection - ad break unavailable", state: state)
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
    
    /// Checks if connection is suitable for high-bitrate streaming
    /// Ethernet and stable WiFi are preferred for CTV
    public func isSuitableForStreaming() -> Bool {
        let state = currentState
        guard state.isConnected else { return false }
        
        // Ethernet is always preferred for CTV
        if state.connectionType == .ethernet {
            return true
        }
        
        // WiFi without constraints is acceptable
        return state.connectionType == .wifi && !state.isConstrained
    }
    
    /// Gets streaming quality hints for adaptive bitrate
    public func streamingQualityHints() -> [String: Any] {
        let state = currentState
        return [
            "isConnected": state.isConnected,
            "connectionType": state.connectionType.rawValue,
            "suggestedQuality": isSuitableForStreaming() ? "high" : "adaptive",
            "shouldBuffer": !isSuitableForStreaming(),
            "fastFailTimeout": Self.offlineFastFailTimeout,
            "normalTimeout": Self.normalTimeout
        ]
    }
    
    /// Gets time since last connectivity change (for rate limiting)
    public func timeSinceLastChange() -> TimeInterval? {
        guard let lastChange = lastConnectivityChange else { return nil }
        return Date().timeIntervalSince(lastChange)
    }
    
    private func updateState(from path: NWPath) {
        let connectionType: ConnectionType
        if path.usesInterfaceType(.wiredEthernet) {
            connectionType = .ethernet
        } else if path.usesInterfaceType(.wifi) {
            connectionType = .wifi
        } else if path.status == .satisfied {
            connectionType = .other
        } else {
            connectionType = .none
        }
        
        lastConnectivityChange = Date()
        
        let isConstrained: Bool
        if #available(tvOS 13.0, macOS 10.15, *) {
            isConstrained = path.isConstrained
        } else {
            isConstrained = false
        }
        
        currentState = NetworkState(
            isConnected: path.status == .satisfied,
            connectionType: connectionType,
            isExpensive: path.isExpensive,
            isConstrained: isConstrained,
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
@available(tvOS 12.0, macOS 10.14, *)
public struct NoNetworkError: Error {
    public let message: String
    public let networkState: NetworkMonitor.NetworkState
    
    public init(message: String, networkState: NetworkMonitor.NetworkState) {
        self.message = message
        self.networkState = networkState
    }
}
