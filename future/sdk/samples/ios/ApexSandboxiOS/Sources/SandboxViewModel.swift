import Foundation
import Combine
import SwiftUI
import RivalApexMediationSDK
#if canImport(AppTrackingTransparency)
import AppTrackingTransparency
#endif

@MainActor
final class SandboxViewModel: ObservableObject {
    // UI state
    @Published var isInitialized: Bool = false
    @Published var interstitialLoaded: Bool = false
    @Published var rewardedLoaded: Bool = false
    @Published var isPresentingAd: Bool = false
    @Published var logs: [String] = []
    @Published var lastError: String?
    @Published var configVersionText: String = "-"
    @Published var attStatusText: String = "NOT_DETERMINED"
    @Published var lastRequestId: String = "-"
    @Published var useInvalidPlacements: Bool = false
    @Published var simulateTimeout: Bool = false
    @Published var isSoakRunning: Bool = false
    @Published var consentGDPR: Bool = false
    @Published var consentCCPA: Bool = false
    @Published var consentCOPPA: Bool = false
    @Published var testMode: Bool = true
    @Published var adapterNamesList: [String] = []
    @Published var selectedAdapter: String? = nil

    // Sandbox configuration
    struct Config: Codable {
        struct Placements: Codable { let interstitialA: String; let rewardedA: String }
        let apiBase: String
        let placements: Placements
        struct Consent: Codable { let gdpr: Bool; let ccpa: Bool; let coppa: Bool; let lat: Bool }
        let consent: Consent?
        let appId: String?
        let adapterWhitelist: [String]?
        let forceAdapterPipeline: Bool?
        let testMode: Bool?
    }
    private(set) var config: Config = .init(apiBase: "https://api.apexmediation.ee/api/v1",
                                            placements: .init(interstitialA: "26c8907d-7635-4a13-a94b-6c3b12af1779",
                                                              rewardedA: "ca610a0c-8607-4a38-8a21-9cbfd3018b7b"),
                                            consent: .init(gdpr: false, ccpa: false, coppa: false, lat: true),
                                            appId: "sandbox-app-ios")

    private var listener: SandboxAdListener!

    init() {
        self.listener = SandboxAdListener(viewModel: self)
        loadSandboxConfig()
        // Seed consent from file
        if let c = config.consent {
            consentGDPR = c.gdpr
            consentCCPA = c.ccpa
            consentCOPPA = c.coppa
        }
        // Apply test mode from config if present
        if let tm = config.testMode { self.testMode = tm }
        // Capture initial ATT status for debug overlay
        Task { @MainActor in
            attStatusText = await ATT.statusDescription()
        }
    }

    func initializeSDK() {
        if isInitialized {
            log("Initialize: already initialized")
            return
        }
        Task { @MainActor in
            do {
                let appId = config.appId ?? "sandbox-app-ios"
                let cfg = SDKConfig(
                    appId: appId,
                    configEndpoint: defaultConfigEndpoint(),
                    auctionEndpoint: defaultAuctionEndpoint(),
                    telemetryEnabled: true,
                    logLevel: .debug,
                    testMode: testMode,
                    configSignaturePublicKey: nil
                )
                try await MediationSDK.shared.initialize(appId: appId, configuration: cfg)
                isInitialized = MediationSDK.shared.isInitialized
                updateConfigVersion()
                applyConsent()
                // In sandbox, exercise adapter pipeline and optionally restrict to a whitelist
                await MediationSDK.shared.setSandboxForceAdapterPipeline(config.forceAdapterPipeline ?? true)
                if let names = config.adapterWhitelist, !names.isEmpty {
                    await MediationSDK.shared.setSandboxAdapterWhitelist(names)
                    log("Sandbox adapter whitelist set: \(names.joined(separator: ", "))")
                }
                let names = await MediationSDK.shared.adapterNames()
                adapterNamesList = names
                selectedAdapter = names.first
                log("Registered adapters: \(names.joined(separator: ", "))")
                log("Initialize: OK (sdkVersion=\(MediationSDK.shared.sdkVersion), testMode=\(MediationSDK.shared.isTestMode))")
            } catch {
                lastError = "Initialize failed: \(error.localizedDescription)"
                log(lastError!)
            }
        }
    }

    func refreshAdapters() {
        Task { @MainActor in
            let names = await MediationSDK.shared.adapterNames()
            adapterNamesList = names
            if selectedAdapter == nil || !(names.contains(selectedAdapter!)) {
                selectedAdapter = names.first
            }
            log("Adapter list refreshed: \(names.joined(separator: ", "))")
        }
    }

    func runSelectedAdapterInterstitial() {
        guard isInitialized else { log("Initialize first"); return }
        guard let name = selectedAdapter else { log("No adapter selected"); return }
        Task { @MainActor in
            await MediationSDK.shared.setSandboxAdapterWhitelist([name])
            log("Running interstitial for adapter=\(name)")
            loadInterstitial()
        }
    }

    func runAllAdapters() {
        guard isInitialized else { log("Initialize first"); return }
        let names = adapterNamesList
        if names.isEmpty { log("No adapters to run"); return }
        Task { @MainActor in
            await MediationSDK.shared.setSandboxForceAdapterPipeline(true)
            for name in names {
                await MediationSDK.shared.setSandboxAdapterWhitelist([name])
                log("[RunAll] Loading interstitial for \(name)")
                loadInterstitial()
                try? await Task.sleep(nanoseconds: 1_000_000_000) // 1s between runs
            }
            await MediationSDK.shared.setSandboxAdapterWhitelist(nil)
            log("[RunAll] Completed")
        }
    }

    func applyConsent() {
        let data = ConsentData(
            gdprApplies: consentGDPR,
            gdprConsentString: consentGDPR ? "TCF_TEST_STRING" : nil,
            ccpaOptOut: consentCCPA,
            coppa: consentCOPPA
        )
        MediationSDK.shared.setConsent(data)
        log("Consent updated: GDPR=\(consentGDPR) CCPA=\(consentCCPA) COPPA=\(consentCOPPA)")
    }

    func loadInterstitial() {
        guard isInitialized else { log("Load Interstitial: SDK not initialized"); return }
        interstitialLoaded = false
        lastRequestId = "int_" + UUID().uuidString.prefix(8)
        let pid = getInterstitialPlacement()
        // Log consent/ATT metadata for verification
        logConsentMetadata()
        if simulateTimeout {
            log("Load Interstitial (simulated timeout) → \(pid)")
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 1_000_000_000) // 1s
                self.listener.onAdFailedToLoad(placementId: pid, error: NSError(domain: "sandbox.timeout", code: -1001, userInfo: [NSLocalizedDescriptionKey: "Simulated timeout"]))
            }
        } else {
            BelInterstitial.load(placementId: pid, listener: listener)
            log("Load Interstitial → \(pid)")
        }
    }

    func showInterstitial() {
        guard isInitialized else { log("Show Interstitial: SDK not initialized"); return }
        guard interstitialLoaded else { log("Show Interstitial: not loaded yet"); return }
        guard let vc = UIApplication.ram_topViewController() else { log("Show Interstitial: no presenter"); return }
        guard !isPresentingAd else { log("Show Interstitial: already presenting; ignored"); return }
        isPresentingAd = true
        _ = BelInterstitial.show(from: vc, placementId: getInterstitialPlacement(), listener: listener)
    }

    func loadRewarded() {
        guard isInitialized else { log("Load Rewarded: SDK not initialized"); return }
        rewardedLoaded = false
        lastRequestId = "rwd_" + UUID().uuidString.prefix(8)
        let pid = getRewardedPlacement()
        // Log consent/ATT metadata for verification
        logConsentMetadata()
        if simulateTimeout {
            log("Load Rewarded (simulated timeout) → \(pid)")
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 1_000_000_000) // 1s
                self.listener.onAdFailedToLoad(placementId: pid, error: NSError(domain: "sandbox.timeout", code: -1001, userInfo: [NSLocalizedDescriptionKey: "Simulated timeout"]))
            }
        } else {
            BelRewarded.load(placementId: pid, listener: listener)
            log("Load Rewarded → \(pid)")
        }
    }

    func showRewarded() {
        guard isInitialized else { log("Show Rewarded: SDK not initialized"); return }
        guard rewardedLoaded else { log("Show Rewarded: not loaded yet"); return }
        guard let vc = UIApplication.ram_topViewController() else { log("Show Rewarded: no presenter"); return }
        guard !isPresentingAd else { log("Show Rewarded: already presenting; ignored"); return }
        isPresentingAd = true
        _ = BelRewarded.show(from: vc, placementId: getRewardedPlacement(), listener: listener)
    }

    func requestATT() async {
        await ATT.requestTrackingAuthorization()
        let status = await ATT.statusDescription()
        attStatusText = status
        log("ATT status: \(status)")
    }

    func updateConfigVersion() {
        if let v = MediationSDK.shared.remoteConfigVersion { configVersionText = "v\(v)" } else { configVersionText = "-" }
    }

    func consentSummaryPretty() -> String {
        let summary = MediationSDK.shared.consentSummary
        let gdpr = (summary["gdpr"] as? Int == 1) ? "1" : "0"
        let ccpa = (summary["us_privacy"] as? String) ?? "-"
        let coppa = (summary["coppa"] as? Int == 1) ? "1" : "0"
        return "GDPR=\(gdpr) CCPA=\(ccpa) COPPA=\(coppa)"
    }

    // MARK: - Helpers
    private func loadSandboxConfig() {
        guard let url = Bundle.main.url(forResource: "SandboxConfig", withExtension: "json") else { return }
        do {
            let data = try Data(contentsOf: url)
            let cfg = try JSONDecoder().decode(Config.self, from: data)
            self.config = cfg
        } catch {
            log("SandboxConfig.json parse failed: \(error.localizedDescription)")
        }
    }

    private func defaultConfigEndpoint() -> String {
        return apiOrigin() ?? SDKConfig.default(appId: "x").configEndpoint
    }

    private func defaultAuctionEndpoint() -> String {
        return apiOrigin() ?? SDKConfig.default(appId: "x").auctionEndpoint
    }

    private func apiOrigin() -> String? {
        guard let url = URL(string: config.apiBase),
              let scheme = url.scheme,
              let host = url.host else { return nil }
        var origin = "\(scheme)://\(host)"
        if let port = url.port { origin += ":\(port)" }
        return origin
    }

    func log(_ line: String) {
        logs.append(line)
        if logs.count > 200 { logs.removeFirst(logs.count - 200) }
        print("[Sandbox] \(line)")
    }

    // MARK: - Test helpers
    private func getInterstitialPlacement() -> String {
        useInvalidPlacements ? "invalid_interstitial_id" : config.placements.interstitialA
    }
    private func getRewardedPlacement() -> String {
        useInvalidPlacements ? "invalid_rewarded_id" : config.placements.rewardedA
    }

    private func logConsentMetadata() {
        let s2s = MediationSDK.shared.consentMetadata()
        let att: String = attStatusText
        // Adapter payload using SDK's ConsentManager
        let adapter = ConsentManager.shared.toAdapterConsentPayload(attStatusProvider: {
            #if canImport(AppTrackingTransparency)
            if #available(iOS 14, *) {
                switch ATTrackingManager.trackingAuthorizationStatus {
                case .authorized: return RivalApexMediationSDK.ATTStatus.authorized
                case .denied: return RivalApexMediationSDK.ATTStatus.denied
                case .restricted: return RivalApexMediationSDK.ATTStatus.restricted
                case .notDetermined: return RivalApexMediationSDK.ATTStatus.notDetermined
                @unknown default: return RivalApexMediationSDK.ATTStatus.unknown
                }
            }
            #endif
            return RivalApexMediationSDK.ATTStatus.notDetermined
        })
        log("Consent S2S: \(s2s)")
        log("Consent Adapter: \(adapter), ATT=\(att)")
    }

    // MARK: - Soak run
    private var soakTask: Task<Void, Never>? = nil

    func startSoakRun(minutes: Int = 30) {
        guard !isSoakRunning else { log("Soak: already running"); return }
        isSoakRunning = true
        let end = Date().addingTimeInterval(Double(minutes) * 60.0)
        soakTask = Task { @MainActor in
            log("Soak: started for \(minutes)m")
            var cycle = 0
            while Date() < end && !Task.isCancelled {
                cycle += 1
                // Interstitial
                loadInterstitial()
                try? await Task.sleep(nanoseconds: 800_000_000)
                if interstitialLoaded { showInterstitial() }
                try? await Task.sleep(nanoseconds: 800_000_000)

                // Rewarded
                loadRewarded()
                try? await Task.sleep(nanoseconds: 800_000_000)
                if rewardedLoaded { showRewarded() }
                try? await Task.sleep(nanoseconds: 800_000_000)

                // Rapid show tap spam simulation (should be debounced)
                if interstitialLoaded {
                    showInterstitial()
                    showInterstitial()
                }

                // Periodic status
                if cycle % 5 == 0 { log("Soak: cycle=\(cycle)") }
            }
            isSoakRunning = false
            log("Soak: finished after \(cycle) cycles")
        }
    }

    func stopSoakRun() {
        soakTask?.cancel()
        soakTask = nil
        isSoakRunning = false
        log("Soak: stopped")
    }
}

// MARK: - Event listener
final class SandboxAdListener: NSObject, BelAdEventListener {
    weak var vm: SandboxViewModel?
    init(viewModel: SandboxViewModel) { self.vm = viewModel }

    func onAdLoaded(placementId: String) {
        Task { @MainActor in
            if placementId == vm?.config.placements.interstitialA { vm?.interstitialLoaded = true }
            if placementId == vm?.config.placements.rewardedA { vm?.rewardedLoaded = true }
            vm?.log("onAdLoaded(\(placementId))")
        }
    }
    func onAdFailedToLoad(placementId: String, error: Error) {
        Task { @MainActor in
            vm?.lastError = error.localizedDescription
            vm?.log("onAdFailedToLoad(\(placementId)): \(error.localizedDescription)")
        }
    }
    func onAdShown(placementId: String) {
        Task { @MainActor in
            vm?.log("onAdShown(\(placementId))")
        }
    }
    func onAdFailedToShow(placementId: String, error: Error) {
        Task { @MainActor in
            vm?.lastError = error.localizedDescription
            vm?.isPresentingAd = false
            vm?.log("onAdFailedToShow(\(placementId)): \(error.localizedDescription)")
        }
    }
    func onAdClicked(placementId: String) {
        Task { @MainActor in vm?.log("onAdClicked(\(placementId))") }
    }
    func onAdClosed(placementId: String) {
        Task { @MainActor in
            vm?.isPresentingAd = false
            if placementId == vm?.config.placements.interstitialA { vm?.interstitialLoaded = false }
            if placementId == vm?.config.placements.rewardedA { vm?.rewardedLoaded = false }
            vm?.log("onAdClosed(\(placementId))")
        }
    }
    func onUserEarnedReward(placementId: String, reward: BelReward?) {
        Task { @MainActor in vm?.log("onUserEarnedReward(\(placementId)): \(reward?.label ?? "reward"):\(reward?.amount ?? 1)") }
    }
}

// MARK: - ATT helper
enum ATT {
    static func requestTrackingAuthorization() async {
        #if canImport(AppTrackingTransparency)
        if #available(iOS 14, *) {
            await withCheckedContinuation { cont in
                ATTrackingManager.requestTrackingAuthorization { _ in cont.resume() }
            }
        }
        #endif
    }
    static func statusDescription() async -> String {
        #if canImport(AppTrackingTransparency)
        if #available(iOS 14, *) {
            switch ATTrackingManager.trackingAuthorizationStatus {
            case .authorized: return "AUTHORIZED"
            case .denied: return "DENIED"
            case .restricted: return "RESTRICTED"
            case .notDetermined: return "NOT_DETERMINED"
            @unknown default: return "UNKNOWN"
            }
        }
        #endif
        return "UNSUPPORTED"
    }
}
