import Foundation

/// Cross-format event listener mirroring the Android BYO callbacks.
public protocol BelAdEventListener: AnyObject {
    func onAdLoaded(placementId: String)
    func onAdFailedToLoad(placementId: String, error: Error)
    func onAdShown(placementId: String)
    func onAdFailedToShow(placementId: String, error: Error)
    func onAdClicked(placementId: String)
    func onAdClosed(placementId: String)
    func onUserEarnedReward(placementId: String, reward: BelReward?)
}

public extension BelAdEventListener {
    func onAdLoaded(placementId: String) {}
    func onAdFailedToLoad(placementId: String, error: Error) {}
    func onAdShown(placementId: String) {}
    func onAdFailedToShow(placementId: String, error: Error) {}
    func onAdClicked(placementId: String) {}
    func onAdClosed(placementId: String) {}
    func onUserEarnedReward(placementId: String, reward: BelReward?) {}
}

/// Simple reward payload for rewarded formats.
public struct BelReward: Equatable {
    public let label: String
    public let amount: Double
    public init(label: String, amount: Double) {
        self.label = label
        self.amount = amount
    }
}
