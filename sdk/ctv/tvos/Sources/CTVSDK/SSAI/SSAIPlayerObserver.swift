import Foundation
import AVFoundation
import Combine

/// SSAI Player Observer for tvOS SDK
///
/// Monitors AVPlayer for SSAI ad playback and automatically tracks events.
@available(tvOS 13.0, *)
public final class SSAIPlayerObserver: NSObject {
    
    // MARK: - Types
    
    public struct ObserverConfig {
        /// Interval for time observation (seconds)
        public let timeObserverInterval: TimeInterval
        
        /// Enable automatic quartile tracking
        public let autoTrackQuartiles: Bool
        
        /// Enable automatic impression tracking
        public let autoTrackImpressions: Bool
        
        /// Tolerance for ad boundary detection (seconds)
        public let boundaryTolerance: TimeInterval
        
        public init(
            timeObserverInterval: TimeInterval = 0.25,
            autoTrackQuartiles: Bool = true,
            autoTrackImpressions: Bool = true,
            boundaryTolerance: TimeInterval = 0.5
        ) {
            self.timeObserverInterval = timeObserverInterval
            self.autoTrackQuartiles = autoTrackQuartiles
            self.autoTrackImpressions = autoTrackImpressions
            self.boundaryTolerance = boundaryTolerance
        }
    }
    
    /// Observer delegate for custom event handling
    public protocol SSAIPlayerObserverDelegate: AnyObject {
        func ssaiObserver(_ observer: SSAIPlayerObserver, didEnterAdBreak adBreak: SSAIAdBreak)
        func ssaiObserver(_ observer: SSAIPlayerObserver, didExitAdBreak adBreak: SSAIAdBreak)
        func ssaiObserver(_ observer: SSAIPlayerObserver, didProgressAd ad: SSAIAd, progress: Double)
        func ssaiObserver(_ observer: SSAIPlayerObserver, didCompleteAd ad: SSAIAd)
        func ssaiObserver(_ observer: SSAIPlayerObserver, didSkipAd ad: SSAIAd)
        func ssaiObserver(_ observer: SSAIPlayerObserver, didEncounterError error: SSAIError, forAd ad: SSAIAd?)
    }
    
    // MARK: - Properties
    
    private weak var player: AVPlayer?
    private let config: ObserverConfig
    private let sessionClient: SSAISessionClient
    private let manifestParser: SSAIManifestParser
    
    public weak var delegate: SSAIPlayerObserverDelegate?
    
    private var timeObserverToken: Any?
    private var boundaryObserverToken: Any?
    private var statusObservation: NSKeyValueObservation?
    private var rateObservation: NSKeyValueObservation?
    
    private var adCues: [SSAIManifestParser.AdCue] = []
    private var currentAdBreak: SSAIAdBreak?
    private var currentAd: SSAIAd?
    private var currentAdProgress: Double = 0
    private var trackedQuartiles: Set<String> = []
    private var impressionsFired: Set<String> = []
    
    private var isPlaying: Bool = false
    private var lastPlaybackTime: TimeInterval = 0
    
    private let adBreakPublisher = PassthroughSubject<SSAIAdBreak, Never>()
    private let adProgressPublisher = PassthroughSubject<(SSAIAd, Double), Never>()
    private var cancellables = Set<AnyCancellable>()
    
    // MARK: - Public Publishers
    
    /// Publisher for ad break entry/exit
    public var adBreakEvents: AnyPublisher<SSAIAdBreak, Never> {
        adBreakPublisher.eraseToAnyPublisher()
    }
    
    /// Publisher for ad progress
    public var adProgressEvents: AnyPublisher<(SSAIAd, Double), Never> {
        adProgressPublisher.eraseToAnyPublisher()
    }
    
    // MARK: - Initialization
    
    public init(
        sessionClient: SSAISessionClient,
        config: ObserverConfig = ObserverConfig()
    ) {
        self.sessionClient = sessionClient
        self.manifestParser = SSAIManifestParser()
        self.config = config
        super.init()
    }
    
    deinit {
        detach()
    }
    
    // MARK: - Public Interface
    
    /// Attach to AVPlayer for observation
    public func attach(to player: AVPlayer) {
        detach()
        self.player = player
        
        setupTimeObserver()
        setupStatusObserver()
        setupRateObserver()
    }
    
    /// Detach from current player
    public func detach() {
        if let token = timeObserverToken, let player = player {
            player.removeTimeObserver(token)
        }
        if let token = boundaryObserverToken, let player = player {
            player.removeTimeObserver(token)
        }
        
        timeObserverToken = nil
        boundaryObserverToken = nil
        statusObservation?.invalidate()
        rateObservation?.invalidate()
        
        player = nil
        currentAdBreak = nil
        currentAd = nil
        currentAdProgress = 0
        trackedQuartiles.removeAll()
        impressionsFired.removeAll()
    }
    
    /// Load ad cues from manifest URL
    public func loadAdCues(from manifestUrl: URL) async throws {
        let result = try await manifestParser.parseHLS(url: manifestUrl)
        adCues = result.adCues
        setupBoundaryObservers()
    }
    
    /// Set ad cues directly
    public func setAdCues(_ cues: [SSAIManifestParser.AdCue]) {
        adCues = cues
        setupBoundaryObservers()
    }
    
    /// Set ad breaks from session
    public func setAdBreaks(_ adBreaks: [SSAIAdBreak]) {
        // Convert ad breaks to cues for boundary detection
        var slotIndex = 0
        adCues = adBreaks.flatMap { adBreak -> [SSAIManifestParser.AdCue] in
            return adBreak.ads.map { ad in
                let cue = SSAIManifestParser.AdCue(
                    id: ad.creativeId,
                    startTime: adBreak.startTime + ad.startOffset,
                    duration: ad.duration,
                    podId: adBreak.podId,
                    slotIndex: slotIndex,
                    cueType: adBreak.type == "preroll" ? .preroll : (adBreak.type == "postroll" ? .postroll : .midroll),
                    metadata: [:]
                )
                slotIndex += 1
                return cue
            }
        }
        
        setupBoundaryObservers()
    }
    
    /// Manually trigger skip (for skip button handling)
    public func skipCurrentAd() {
        guard let ad = currentAd, let adBreak = currentAdBreak else { return }
        
        sessionClient.trackSkip(
            podId: adBreak.podId,
            slotId: ad.slotId,
            creativeId: ad.creativeId,
            mediaTime: lastPlaybackTime
        )
        
        delegate?.ssaiObserver(self, didSkipAd: ad)
        
        // Move to next ad or exit ad break
        currentAd = nil
        currentAdProgress = 0
    }
    
    /// Check if currently in ad break
    public var isInAdBreak: Bool {
        currentAdBreak != nil
    }
    
    /// Get current ad info
    public var currentAdInfo: SSAIAd? {
        currentAd
    }
    
    /// Get current ad break info
    public var currentAdBreakInfo: SSAIAdBreak? {
        currentAdBreak
    }
    
    // MARK: - Private Methods
    
    private func setupTimeObserver() {
        guard let player = player else { return }
        
        let interval = CMTime(seconds: config.timeObserverInterval, preferredTimescale: CMTimeScale(NSEC_PER_SEC))
        
        timeObserverToken = player.addPeriodicTimeObserver(forInterval: interval, queue: .main) { [weak self] time in
            self?.handleTimeUpdate(time: time.seconds)
        }
    }
    
    private func setupBoundaryObservers() {
        guard let player = player, !adCues.isEmpty else { return }
        
        // Remove existing boundary observer
        if let token = boundaryObserverToken {
            player.removeTimeObserver(token)
            boundaryObserverToken = nil
        }
        
        // Create boundary times for ad starts
        let startTimes = adCues.map { NSValue(time: CMTime(seconds: $0.startTime, preferredTimescale: CMTimeScale(NSEC_PER_SEC))) }
        
        // Create boundary times for ad ends
        let endTimes = adCues.map { NSValue(time: CMTime(seconds: $0.startTime + $0.duration, preferredTimescale: CMTimeScale(NSEC_PER_SEC))) }
        
        let allTimes = startTimes + endTimes
        
        boundaryObserverToken = player.addBoundaryTimeObserver(forTimes: allTimes, queue: .main) { [weak self] in
            self?.handleBoundaryReached()
        }
    }
    
    private func setupStatusObserver() {
        guard let player = player else { return }
        
        statusObservation = player.observe(\.status, options: [.new]) { [weak self] player, _ in
            if player.status == .failed {
                let error = SSAIError(
                    code: "PLAYER_ERROR",
                    message: player.error?.localizedDescription ?? "Unknown player error",
                    fatal: true
                )
                self?.delegate?.ssaiObserver(self!, didEncounterError: error, forAd: self?.currentAd)
            }
        }
    }
    
    private func setupRateObserver() {
        guard let player = player else { return }
        
        rateObservation = player.observe(\.rate, options: [.new]) { [weak self] player, _ in
            self?.isPlaying = player.rate > 0
        }
    }
    
    private func handleTimeUpdate(time: TimeInterval) {
        lastPlaybackTime = time
        
        // Check if we're in an ad
        let currentCue = findCurrentCue(at: time)
        
        if let cue = currentCue {
            handleAdPlayback(cue: cue, currentTime: time)
        } else if currentAdBreak != nil {
            exitCurrentAdBreak()
        }
    }
    
    private func handleBoundaryReached() {
        // This is called at ad boundaries - can be used for precise timing
        guard let player = player else { return }
        let currentTime = player.currentTime().seconds
        
        if let cue = findCurrentCue(at: currentTime) {
            // Just entered an ad
            if currentAd == nil {
                enterAd(cue: cue)
            }
        }
    }
    
    private func findCurrentCue(at time: TimeInterval) -> SSAIManifestParser.AdCue? {
        for cue in adCues {
            let adStart = cue.startTime - config.boundaryTolerance
            let adEnd = cue.startTime + cue.duration + config.boundaryTolerance
            
            if time >= adStart && time <= adEnd {
                return cue
            }
        }
        return nil
    }
    
    private func handleAdPlayback(cue: SSAIManifestParser.AdCue, currentTime: TimeInterval) {
        // Create ad break if needed
        if currentAdBreak == nil {
            enterAdBreak(cue: cue)
        }
        
        // Create current ad if needed
        if currentAd == nil {
            enterAd(cue: cue)
        }
        
        // Calculate progress
        guard let ad = currentAd else { return }
        let adStartTime = cue.startTime
        let adDuration = cue.duration
        
        guard adDuration > 0 else { return }
        
        let adElapsed = currentTime - adStartTime
        let progress = min(100, max(0, (adElapsed / adDuration) * 100))
        
        currentAdProgress = progress
        adProgressPublisher.send((ad, progress))
        delegate?.ssaiObserver(self, didProgressAd: ad, progress: progress)
        
        // Auto-track quartiles
        if config.autoTrackQuartiles {
            trackQuartilesIfNeeded(ad: ad, progress: progress, cue: cue)
        }
        
        // Check for ad completion
        if progress >= 100 {
            completeCurrentAd(cue: cue)
        }
    }
    
    private func enterAdBreak(cue: SSAIManifestParser.AdCue) {
        // Find all ads in this pod
        let podAds = adCues.filter { $0.podId == cue.podId }
        
        let ads = podAds.map { podCue -> SSAIAd in
            SSAIAd(
                slotId: "slot-\(podCue.slotIndex)",
                creativeId: podCue.id,
                duration: podCue.duration,
                startOffset: podCue.startTime - cue.startTime,
                skipOffset: nil,
                vastUrl: nil,
                verificationScripts: []
            )
        }
        
        let totalDuration = podAds.reduce(0) { $0 + $1.duration }
        
        let adBreak = SSAIAdBreak(
            podId: cue.podId,
            type: cue.cueType.rawValue,
            position: Int(cue.startTime),
            startTime: cue.startTime,
            duration: totalDuration,
            plannedSlots: ads.count,
            filledSlots: ads.count,
            ads: ads
        )
        
        currentAdBreak = adBreak
        adBreakPublisher.send(adBreak)
        delegate?.ssaiObserver(self, didEnterAdBreak: adBreak)
    }
    
    private func enterAd(cue: SSAIManifestParser.AdCue) {
        let ad = SSAIAd(
            slotId: "slot-\(cue.slotIndex)",
            creativeId: cue.id,
            duration: cue.duration,
            startOffset: 0,
            skipOffset: nil,
            vastUrl: nil,
            verificationScripts: []
        )
        
        currentAd = ad
        currentAdProgress = 0
        
        // Track impression
        if config.autoTrackImpressions {
            let impressionKey = "\(cue.podId):\(ad.creativeId)"
            if !impressionsFired.contains(impressionKey) {
                impressionsFired.insert(impressionKey)
                
                sessionClient.trackImpression(
                    podId: cue.podId,
                    slotId: ad.slotId,
                    creativeId: ad.creativeId,
                    mediaTime: cue.startTime
                )
            }
        }
    }
    
    private func trackQuartilesIfNeeded(ad: SSAIAd, progress: Double, cue: SSAIManifestParser.AdCue) {
        guard let adBreak = currentAdBreak else { return }
        
        let quartiles: [(threshold: Double, type: SSAITrackingEventType)] = [
            (0, .start),
            (25, .firstQuartile),
            (50, .midpoint),
            (75, .thirdQuartile)
        ]
        
        for (threshold, eventType) in quartiles {
            let quartileKey = "\(adBreak.podId):\(ad.creativeId):\(eventType.rawValue)"
            
            if progress >= threshold && !trackedQuartiles.contains(quartileKey) {
                trackedQuartiles.insert(quartileKey)
                
                sessionClient.trackProgress(
                    podId: adBreak.podId,
                    slotId: ad.slotId,
                    creativeId: ad.creativeId,
                    mediaTime: lastPlaybackTime,
                    progress: threshold
                )
            }
        }
    }
    
    private func completeCurrentAd(cue: SSAIManifestParser.AdCue) {
        guard let ad = currentAd, let adBreak = currentAdBreak else { return }
        
        // Track complete
        let completeKey = "\(adBreak.podId):\(ad.creativeId):complete"
        if !trackedQuartiles.contains(completeKey) {
            trackedQuartiles.insert(completeKey)
            
            sessionClient.trackProgress(
                podId: adBreak.podId,
                slotId: ad.slotId,
                creativeId: ad.creativeId,
                mediaTime: lastPlaybackTime,
                progress: 100
            )
        }
        
        delegate?.ssaiObserver(self, didCompleteAd: ad)
        
        // Check if there are more ads in the pod
        let currentIndex = cue.slotIndex
        let nextCue = adCues.first { $0.podId == cue.podId && $0.slotIndex == currentIndex + 1 }
        
        if nextCue == nil {
            // No more ads, exit ad break
            exitCurrentAdBreak()
        } else {
            // More ads, clear current ad for next one
            currentAd = nil
            currentAdProgress = 0
        }
    }
    
    private func exitCurrentAdBreak() {
        guard let adBreak = currentAdBreak else { return }
        
        delegate?.ssaiObserver(self, didExitAdBreak: adBreak)
        
        currentAdBreak = nil
        currentAd = nil
        currentAdProgress = 0
    }
}

// MARK: - SSAIPlayerObserverDelegate Default Implementation

@available(tvOS 13.0, *)
public extension SSAIPlayerObserver.SSAIPlayerObserverDelegate {
    func ssaiObserver(_ observer: SSAIPlayerObserver, didEnterAdBreak adBreak: SSAIAdBreak) {}
    func ssaiObserver(_ observer: SSAIPlayerObserver, didExitAdBreak adBreak: SSAIAdBreak) {}
    func ssaiObserver(_ observer: SSAIPlayerObserver, didProgressAd ad: SSAIAd, progress: Double) {}
    func ssaiObserver(_ observer: SSAIPlayerObserver, didCompleteAd ad: SSAIAd) {}
    func ssaiObserver(_ observer: SSAIPlayerObserver, didSkipAd ad: SSAIAd) {}
    func ssaiObserver(_ observer: SSAIPlayerObserver, didEncounterError error: SSAIError, forAd ad: SSAIAd?) {}
}
