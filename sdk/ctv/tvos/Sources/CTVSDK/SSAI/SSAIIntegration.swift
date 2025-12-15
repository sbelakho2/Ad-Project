import Foundation
import AVFoundation
import Combine

/// SSAI Integration for tvOS SDK
///
/// Main entry point for SSAI functionality. Provides a unified interface
/// for session management, manifest parsing, and player observation.
@available(tvOS 13.0, *)
public final class SSAIIntegration {
    
    // MARK: - Singleton
    
    /// Shared instance for convenience
    public static let shared = SSAIIntegration()
    
    // MARK: - Properties
    
    private var sessionClient: SSAISessionClient?
    private var playerObserver: SSAIPlayerObserver?
    private var manifestParser: SSAIManifestParser?
    
    private var currentSession: SSAISession?
    private var currentConfig: SSAIConfig?
    private var isInitialized = false
    
    private var cancellables = Set<AnyCancellable>()
    
    // MARK: - Public Publishers
    
    private let adBreakSubject = PassthroughSubject<SSAIAdBreak, Never>()
    private let adProgressSubject = PassthroughSubject<(SSAIAd, Double), Never>()
    private let errorSubject = PassthroughSubject<SSAIError, Never>()
    
    /// Publisher for ad break events
    public var adBreakEvents: AnyPublisher<SSAIAdBreak, Never> {
        adBreakSubject.eraseToAnyPublisher()
    }
    
    /// Publisher for ad progress events
    public var adProgressEvents: AnyPublisher<(SSAIAd, Double), Never> {
        adProgressSubject.eraseToAnyPublisher()
    }
    
    /// Publisher for errors
    public var errorEvents: AnyPublisher<SSAIError, Never> {
        errorSubject.eraseToAnyPublisher()
    }
    
    // MARK: - Initialization
    
    public init() {}
    
    // MARK: - Public Interface
    
    /// Initialize SSAI integration with configuration
    /// - Parameters:
    ///   - config: SSAI configuration
    ///   - clientConfig: Optional custom session client configuration
    ///   - observerConfig: Optional custom observer configuration
    public func initialize(
        config: SSAIConfig,
        clientConfig: SSAISessionClient.Config = SSAISessionClient.Config(),
        observerConfig: SSAIPlayerObserver.ObserverConfig = SSAIPlayerObserver.ObserverConfig()
    ) async throws -> SSAISession {
        currentConfig = config
        
        // Create session client
        let client = SSAISessionClient(config: clientConfig)
        sessionClient = client
        
        // Create manifest parser
        manifestParser = SSAIManifestParser(debug: config.debug)
        
        // Create session
        let session = try await client.createSession(config: config)
        currentSession = session
        
        // Create player observer
        let observer = SSAIPlayerObserver(sessionClient: client, config: observerConfig)
        playerObserver = observer
        
        // Forward observer events
        observer.adBreakEvents
            .sink { [weak self] adBreak in
                self?.adBreakSubject.send(adBreak)
            }
            .store(in: &cancellables)
        
        observer.adProgressEvents
            .sink { [weak self] (ad, progress) in
                self?.adProgressSubject.send((ad, progress))
            }
            .store(in: &cancellables)
        
        isInitialized = true
        
        if config.debug {
            print("[SSAI] Initialized with session: \(session.sessionId)")
        }
        
        return session
    }
    
    /// Initialize with completion handler (for Objective-C compatibility)
    public func initialize(
        config: SSAIConfig,
        completion: @escaping (Result<SSAISession, SSAIError>) -> Void
    ) {
        Task {
            do {
                let session = try await initialize(config: config)
                DispatchQueue.main.async { completion(.success(session)) }
            } catch let error as SSAIError {
                DispatchQueue.main.async { completion(.failure(error)) }
            } catch {
                let ssaiError = SSAIError(code: "INIT_ERROR", message: error.localizedDescription, fatal: true)
                DispatchQueue.main.async { completion(.failure(ssaiError)) }
            }
        }
    }
    
    /// Attach to AVPlayer for automatic tracking
    /// - Parameter player: The AVPlayer instance to observe
    public func attach(to player: AVPlayer) {
        guard let observer = playerObserver else {
            if currentConfig?.debug == true {
                print("[SSAI] Cannot attach - not initialized")
            }
            return
        }
        
        observer.attach(to: player)
        
        // Load ad breaks from session if available
        if let session = currentSession {
            observer.setAdBreaks(session.adBreaks)
        }
    }
    
    /// Detach from player
    public func detach() {
        playerObserver?.detach()
    }
    
    /// Set observer delegate for custom handling
    public func setObserverDelegate(_ delegate: SSAIPlayerObserver.SSAIPlayerObserverDelegate) {
        playerObserver?.delegate = delegate
    }
    
    /// Get manifest URL with SSAI parameters
    public var manifestUrl: URL? {
        currentSession?.manifestUrl
    }
    
    /// Get current session info
    public var session: SSAISession? {
        currentSession
    }
    
    /// Get session metrics
    public var metrics: SSAIMetrics {
        sessionClient?.currentMetrics ?? SSAIMetrics()
    }
    
    /// Check if currently in ad break
    public var isInAdBreak: Bool {
        playerObserver?.isInAdBreak ?? false
    }
    
    /// Get current ad info
    public var currentAd: SSAIAd? {
        playerObserver?.currentAdInfo
    }
    
    /// Get current ad break info
    public var currentAdBreak: SSAIAdBreak? {
        playerObserver?.currentAdBreakInfo
    }
    
    /// Track a custom event
    public func trackEvent(_ event: SSAITrackingEvent) {
        sessionClient?.trackEvent(event)
    }
    
    /// Skip current ad (for skip button)
    public func skipCurrentAd() {
        playerObserver?.skipCurrentAd()
    }
    
    /// Refresh session (for long-running streams)
    public func refreshSession() async throws -> SSAISession {
        guard let config = currentConfig else {
            throw SSAIError(code: "NOT_INITIALIZED", message: "SSAI not initialized", fatal: true)
        }
        
        // End current session
        sessionClient?.endSession()
        
        // Create new session
        return try await initialize(config: config)
    }
    
    /// End session and cleanup
    public func shutdown() {
        playerObserver?.detach()
        sessionClient?.endSession()
        
        sessionClient = nil
        playerObserver = nil
        manifestParser = nil
        currentSession = nil
        currentConfig = nil
        isInitialized = false
        cancellables.removeAll()
    }
    
    // MARK: - Convenience Methods
    
    /// Create an AVPlayerItem with SSAI manifest
    public func createPlayerItem() -> AVPlayerItem? {
        guard let manifestUrl = currentSession?.manifestUrl else { return nil }
        return AVPlayerItem(url: manifestUrl)
    }
    
    /// Parse manifest for ad cues (for custom implementations)
    public func parseManifest(url: URL) async throws -> SSAIManifestParser.ParseResult {
        guard let parser = manifestParser else {
            throw SSAIError(code: "NOT_INITIALIZED", message: "SSAI not initialized", fatal: true)
        }
        return try await parser.parseHLS(url: url)
    }
}

// MARK: - Convenience Factory

@available(tvOS 13.0, *)
public extension SSAIIntegration {
    
    /// Create a new SSAI integration instance (non-singleton)
    static func create(
        config: SSAIConfig,
        clientConfig: SSAISessionClient.Config = SSAISessionClient.Config(),
        observerConfig: SSAIPlayerObserver.ObserverConfig = SSAIPlayerObserver.ObserverConfig()
    ) async throws -> SSAIIntegration {
        let integration = SSAIIntegration()
        _ = try await integration.initialize(config: config, clientConfig: clientConfig, observerConfig: observerConfig)
        return integration
    }
}

// MARK: - SSAI Configuration Builder

public extension SSAIConfig {
    
    /// Builder for creating SSAI configurations
    final class Builder {
        private var endpoint: URL?
        private var tenantId: String?
        private var contentUrl: URL?
        private var format: SSAIContentFormat = .hls
        private var capabilities: SSAIDeviceCapabilities?
        private var timeoutSeconds: TimeInterval = 10.0
        private var debug: Bool = false
        
        public init() {}
        
        @discardableResult
        public func setEndpoint(_ endpoint: URL) -> Builder {
            self.endpoint = endpoint
            return self
        }
        
        @discardableResult
        public func setEndpoint(_ endpoint: String) -> Builder {
            self.endpoint = URL(string: endpoint)
            return self
        }
        
        @discardableResult
        public func setTenantId(_ tenantId: String) -> Builder {
            self.tenantId = tenantId
            return self
        }
        
        @discardableResult
        public func setContentUrl(_ contentUrl: URL) -> Builder {
            self.contentUrl = contentUrl
            return self
        }
        
        @discardableResult
        public func setContentUrl(_ contentUrl: String) -> Builder {
            self.contentUrl = URL(string: contentUrl)
            return self
        }
        
        @discardableResult
        public func setFormat(_ format: SSAIContentFormat) -> Builder {
            self.format = format
            return self
        }
        
        @discardableResult
        public func setCapabilities(_ capabilities: SSAIDeviceCapabilities) -> Builder {
            self.capabilities = capabilities
            return self
        }
        
        @discardableResult
        public func useDefaultTvOSCapabilities() -> Builder {
            self.capabilities = .tvOS
            return self
        }
        
        @discardableResult
        public func setTimeout(_ seconds: TimeInterval) -> Builder {
            self.timeoutSeconds = seconds
            return self
        }
        
        @discardableResult
        public func setDebug(_ debug: Bool) -> Builder {
            self.debug = debug
            return self
        }
        
        public func build() throws -> SSAIConfig {
            guard let endpoint = endpoint else {
                throw SSAIError(code: "MISSING_ENDPOINT", message: "Endpoint is required", fatal: true)
            }
            guard let tenantId = tenantId else {
                throw SSAIError(code: "MISSING_TENANT_ID", message: "Tenant ID is required", fatal: true)
            }
            guard let contentUrl = contentUrl else {
                throw SSAIError(code: "MISSING_CONTENT_URL", message: "Content URL is required", fatal: true)
            }
            
            return SSAIConfig(
                endpoint: endpoint,
                tenantId: tenantId,
                contentUrl: contentUrl,
                format: format,
                capabilities: capabilities ?? .tvOS,
                timeoutSeconds: timeoutSeconds,
                debug: debug
            )
        }
    }
    
    /// Create a new builder
    static func builder() -> Builder {
        Builder()
    }
}
