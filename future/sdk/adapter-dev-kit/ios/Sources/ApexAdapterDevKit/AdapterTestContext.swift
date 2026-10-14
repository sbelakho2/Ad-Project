import Foundation
import RivalApexMediationSDK

public struct AdapterTestContext {
    public let appId: String
    public let placementInterstitial: String
    public let placementRewarded: String
    public let timeout: TimeInterval

    public init(appId: String,
                placementInterstitial: String,
                placementRewarded: String,
                timeout: TimeInterval = 10.0) {
        self.appId = appId
        self.placementInterstitial = placementInterstitial
        self.placementRewarded = placementRewarded
        self.timeout = timeout
    }
}
