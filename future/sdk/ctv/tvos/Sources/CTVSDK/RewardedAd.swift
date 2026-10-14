#if canImport(AVKit) && canImport(UIKit)
import AVKit
import UIKit

public final class RewardedAd {
    private let placementId: String
    private var win: AuctionWin?
    private var loaded = false

    public init(placementId: String) { self.placementId = placementId }

    public func load(floorCpm: Double = 0.0, completion: @escaping (_ error: String?) -> Void) {
        let sdk = ApexMediation.shared
        guard sdk.isInitialized else { completion("not_initialized"); return }
        if let reason = sdk.loadGuard(for: placementId) { completion(reason); return }
        sdk.client.requestBid(placementId: placementId, adFormat: "rewarded", floorCpm: floorCpm, consent: sdk.consent) { res in
            if let error = res.error {
                sdk.recordLoadFailure(placementId: self.placementId, reason: error.reason)
                completion(error.reason)
                return
            }
            guard let win = res.win else {
                sdk.recordLoadFailure(placementId: self.placementId, reason: "no_fill")
                completion("no_fill")
                return
            }
            AdCache.shared.store(win: win, for: self.placementId)
            self.win = win
            self.loaded = true
            sdk.recordLoadSuccess(placementId: self.placementId)
            completion(nil)
        }
    }

    public var isReady: Bool { AdCache.shared.peek(placementId: placementId) != nil }

    public func show(on viewController: UIViewController, completion: @escaping (_ error: String?) -> Void, closed: @escaping () -> Void) {
        guard let cachedWin = AdCache.shared.take(placementId: placementId) else {
            if loaded {
                loaded = false
                completion("expired")
            } else {
                completion("not_ready")
            }
            return
        }
        win = cachedWin
        loaded = false
        if let reason = ApexMediation.shared.showGuard(for: placementId) { completion(reason); return }
        let player = AVPlayer(url: URL(string: cachedWin.creativeUrl)!)
        let controller = AVPlayerViewController()
        controller.player = player
        controller.modalPresentationStyle = .fullScreen
        let tracker = VideoTracker(tracking: cachedWin.tracking)
        viewController.present(controller, animated: true) {
            tracker.attach(to: player)
            Beacon.fire(cachedWin.tracking.impression, eventName: "impression")
            controller.player?.play()
            completion(nil)
        }
        var observer: NSObjectProtocol?
        observer = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: player.currentItem, queue: .main) { _ in
            if let obs = observer { NotificationCenter.default.removeObserver(obs) }
            tracker.markComplete()
            tracker.markClose()
            controller.dismiss(animated: true) {
                tracker.stop()
                closed()
            }
        }
    }

    /// Report a manual click interaction for the current ad (fires signed beacon).
    public func reportClick() {
        guard let current = win else { return }
        Beacon.fire(current.tracking.click, eventName: "click")
    }
}
#else
public final class RewardedAd {
    public init(placementId: String) {}
    public func load(floorCpm: Double = 0.0, completion: @escaping (_ error: String?) -> Void) { completion("unsupported_platform") }
    public var isReady: Bool { false }
    public func show(on viewController: AnyObject, completion: @escaping (_ error: String?) -> Void, closed: @escaping () -> Void) { completion("unsupported_platform") }
    public func reportClick() {}
}
#endif
