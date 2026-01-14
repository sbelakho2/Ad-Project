import Foundation
import SwiftUI
import RivalApexMediationSDK

@MainActor
final class SandboxViewModel: ObservableObject {
    @Published var initialized = false
    @Published var isPresentingAd = false
    @Published var lastError: String?
    @Published var logLines: [String] = []

    struct Config: Codable {
        struct Placements: Codable { let interstitialA: String; let rewardedA: String }
        let appId: String
        let placements: Placements
        let testMode: Bool
        let adapterWhitelist: [String]?
        let forceAdapterPipeline: Bool?
    }

    private(set) var config: Config = .init(
        appId: "sandbox-app-tvos",
        placements: .init(
            interstitialA: "8a94e8fd-8995-4b17-b308-38f1104d1e84",
            rewardedA: "090ce110-b9dc-4722-93dd-8bd3fcfdb2e0"
        ),
        testMode: true
    )

    init() {
        loadConfig()
        log("app_start platform=tvos")
    }

    func initializeSDK() {
        guard !initialized else { log("initialize already platform=tvos"); return }
        do {
            let cfg = SDKConfig(
                appId: config.appId,
                configEndpoint: SDKConfig.default(appId: config.appId).configEndpoint,
                auctionEndpoint: SDKConfig.default(appId: config.appId).auctionEndpoint,
                telemetryEnabled: true,
                logLevel: .debug,
                testMode: config.testMode,
                configSignaturePublicKey: nil
            )
            try MediationSDK.shared.initializeSync(appId: config.appId, configuration: cfg)
            initialized = MediationSDK.shared.isInitialized
            Task { @MainActor in
                await MediationSDK.shared.setSandboxForceAdapterPipeline(config.forceAdapterPipeline ?? true)
                if let names = config.adapterWhitelist, !names.isEmpty {
                    await MediationSDK.shared.setSandboxAdapterWhitelist(names)
                    log("adapter_whitelist set=\(names.joined(separator: ",")) platform=tvos")
                }
                let names = await MediationSDK.shared.adapterNames()
                log("adapters registered=\(names.joined(separator: ",")) platform=tvos")
            }
            log("initialize ok platform=tvos")
        } catch {
            lastError = "Initialize failed: \(error.localizedDescription)"
            log("initialize error=\(error.localizedDescription) platform=tvos")
        }
    }

    func showInterstitial() {
        guard initialized else { log("show_interstitial skipped reason=not_initialized platform=tvos"); return }
        guard !isPresentingAd else { log("show_interstitial skipped reason=presenting platform=tvos"); return }
        guard let vc = UIApplication.ram_topViewController() else { log("show_interstitial skipped reason=no_presenter platform=tvos"); return }
        isPresentingAd = true
        log("show_interstitial platform=tvos")
        _ = BelInterstitial.show(from: vc, placementId: config.placements.interstitialA, listener: self)
    }

    func showRewarded() {
        guard initialized else { log("show_rewarded skipped reason=not_initialized platform=tvos"); return }
        guard !isPresentingAd else { log("show_rewarded skipped reason=presenting platform=tvos"); return }
        guard let vc = UIApplication.ram_topViewController() else { log("show_rewarded skipped reason=no_presenter platform=tvos"); return }
        isPresentingAd = true
        log("show_rewarded platform=tvos")
        _ = BelRewarded.show(from: vc, placementId: config.placements.rewardedA, listener: self)
    }

    func onBackground() { log("lifecycle background platform=tvos") }
    func onForeground() { log("lifecycle foreground platform=tvos") }

    private func loadConfig() {
        guard let url = Bundle.main.url(forResource: "SandboxConfig", withExtension: "json") else { return }
        do {
            let data = try Data(contentsOf: url)
            let cfg = try JSONDecoder().decode(Config.self, from: data)
            self.config = cfg
        } catch {
            log("config_parse_fail error=\(error.localizedDescription) platform=tvos")
        }
    }

    func log(_ line: String) {
        logLines.append(line)
        if logLines.count > 200 { logLines.removeFirst(logLines.count - 200) }
        print("[ApexSandboxTV] \(line)")
    }
}

extension SandboxViewModel: BelAdEventListener {
    func onAdLoaded(placementId: String) { log("onAdLoaded pid=\(placementId) platform=tvos") }
    func onAdFailedToLoad(placementId: String, error: Error) { lastError = error.localizedDescription; log("onAdFailedToLoad pid=\(placementId) err=\(error.localizedDescription) platform=tvos") }
    func onAdShown(placementId: String) { log("onAdShown pid=\(placementId) platform=tvos") }
    func onAdFailedToShow(placementId: String, error: Error) { lastError = error.localizedDescription; isPresentingAd = false; log("onAdFailedToShow pid=\(placementId) err=\(error.localizedDescription) platform=tvos") }
    func onAdClicked(placementId: String) { log("onAdClicked pid=\(placementId) platform=tvos") }
    func onAdClosed(placementId: String) { isPresentingAd = false; log("onAdClosed pid=\(placementId) platform=tvos") }
    func onUserEarnedReward(placementId: String, reward: BelReward?) { log("onUserEarnedReward pid=\(placementId) amount=\(reward?.amount ?? 1) platform=tvos") }
}
