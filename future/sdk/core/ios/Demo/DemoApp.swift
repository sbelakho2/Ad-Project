//
//  DemoApp.swift
//  RivalApexMediationSDK Demo
//
//  Section 3.1: iOS Demo App Target with MockURLProtocol
//  Demonstrates interstitial/rewarded flows with deterministic scenarios:
//  - Success (200)
//  - No fill (204)
//  - Rate limit (429)
//  - Server error (503)
//  - Timeout

import SwiftUI
import RivalApexMediationSDK

@main
struct DemoApp: App {
    init() {
        // Register MockURLProtocol for deterministic testing
        URLProtocol.registerClass(MockURLProtocol.self)
    }
    
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    @StateObject private var viewModel = DemoViewModel()
    
    var body: some View {
        NavigationView {
            VStack(spacing: 20) {
                Text("Rival Apex Mediation SDK")
                    .font(.title)
                    .padding()
                
                // SDK Status
                GroupBox(label: Text("SDK Status")) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Initialized: \(viewModel.isInitialized ? "✅" : "❌")")
                        Text("Test Mode: ON")
                        Text("Version: \(viewModel.sdkVersion)")
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                
                // Initialize SDK
                if !viewModel.isInitialized {
                    Button("Initialize SDK") {
                        Task {
                            await viewModel.initializeSDK()
                        }
                    }
                    .buttonStyle(.borderedProminent)
                }
                
                Divider()
                
                // Interstitial Section
                GroupBox(label: Text("Interstitial Ads")) {
                    VStack(spacing: 12) {
                        ForEach(DemoScenario.allCases, id: \.self) { scenario in
                            Button("Load \(scenario.displayName)") {
                                Task {
                                    await viewModel.loadInterstitial(scenario: scenario)
                                }
                            }
                            .buttonStyle(.bordered)
                            .disabled(!viewModel.isInitialized)
                        }
                    }
                }
                
                // Rewarded Section
                GroupBox(label: Text("Rewarded Ads")) {
                    VStack(spacing: 12) {
                        ForEach(DemoScenario.allCases, id: \.self) { scenario in
                            Button("Load \(scenario.displayName)") {
                                Task {
                                    await viewModel.loadRewarded(scenario: scenario)
                                }
                            }
                            .buttonStyle(.bordered)
                            .disabled(!viewModel.isInitialized)
                        }
                    }
                }
                
                // Results
                if !viewModel.lastResult.isEmpty {
                    GroupBox(label: Text("Last Result")) {
                        ScrollView {
                            Text(viewModel.lastResult)
                                .font(.system(.caption, design: .monospaced))
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .frame(maxHeight: 150)
                    }
                }
                
                Spacer()
                
                // Debug Panel
                Button("Open Debug Panel") {
                    viewModel.showDebugPanel = true
                }
                .buttonStyle(.borderless)
            }
            .padding()
            .navigationTitle("SDK Demo")
            .alert("Debug Panel", isPresented: $viewModel.showDebugPanel) {
                Button("Close", role: .cancel) {}
            } message: {
                Text(viewModel.debugInfo)
            }
        }
    }
}

// MARK: - ViewModel

@MainActor
class DemoViewModel: ObservableObject {
    @Published var isInitialized = false
    @Published var lastResult = ""
    @Published var showDebugPanel = false
    
    let sdkVersion = "1.0.0"
    
    var debugInfo: String {
        """
        Apex Mediation SDK Demo
        Version: \(sdkVersion)
        Initialized: \(isInitialized)
        Test Mode: ON
        Mock URL Protocol: Active
        """
    }
    
    func initializeSDK() async {
        lastResult = "Initializing SDK..."
        
        do {
            let config = SDKConfig(
                appId: "demo_app_123",
                endpoints: SDKConfig.Endpoints(
                    configUrl: URL(string: "https://mock-config.rival-apex.com/v1/config")!,
                    auctionUrl: URL(string: "https://mock-auction.rival-apex.com/v1/auction")!
                ),
                telemetryEnabled: false,
                logLevel: .debug,
                testMode: true,
                configSignaturePublicKey: nil
            )
            
            try await MediationSDK.shared.initialize(appId: "demo_app_123", configuration: config)
            isInitialized = true
            lastResult = "✅ SDK initialized successfully"
        } catch {
            lastResult = "❌ Initialization failed: \(error.localizedDescription)"
        }
    }
    
    func loadInterstitial(scenario: DemoScenario) async {
        MockURLProtocol.currentScenario = scenario
        lastResult = "Loading interstitial (\(scenario.displayName))..."
        
        do {
            let ad = try await MediationSDK.shared.loadAd(placementId: "interstitial_main")
            if let ad = ad {
                lastResult = """
                ✅ Interstitial loaded
                Ad ID: \(ad.adId)
                Network: \(ad.networkName)
                CPM: $\(ad.cpm)
                Type: \(ad.adType)
                """
            } else {
                lastResult = "⚠️ No ad returned"
            }
        } catch let error as SDKError {
            lastResult = "❌ \(formatError(error))"
        } catch {
            lastResult = "❌ Unexpected error: \(error.localizedDescription)"
        }
    }
    
    func loadRewarded(scenario: DemoScenario) async {
        MockURLProtocol.currentScenario = scenario
        lastResult = "Loading rewarded (\(scenario.displayName))..."
        
        do {
            let ad = try await MediationSDK.shared.loadAd(placementId: "rewarded_main")
            if let ad = ad {
                lastResult = """
                ✅ Rewarded ad loaded
                Ad ID: \(ad.adId)
                Network: \(ad.networkName)
                CPM: $\(ad.cpm)
                Type: \(ad.adType)
                """
            } else {
                lastResult = "⚠️ No ad returned"
            }
        } catch let error as SDKError {
            lastResult = "❌ \(formatError(error))"
        } catch {
            lastResult = "❌ Unexpected error: \(error.localizedDescription)"
        }
    }
    
    private func formatError(_ error: SDKError) -> String {
        switch error {
        case .noFill:
            return "No Fill - No ad inventory available"
        case .status_429(let message):
            return "Rate Limit (429) - \(message)"
        case .status_5xx(let code, let message):
            return "Server Error (\(code)) - \(message)"
        case .timeout:
            return "Timeout - Request exceeded time budget"
        case .networkError(let underlying):
            return "Network Error - \(underlying ?? "Unknown")"
        case .notInitialized:
            return "Not Initialized - Call initialize() first"
        case .invalidPlacement(let id):
            return "Invalid Placement - \(id)"
        default:
            return error.localizedDescription
        }
    }
}

// MARK: - Test Scenarios

enum DemoScenario: CaseIterable {
    case success
    case noFill
    case rateLimitExceeded
    case serverError
    case timeout
    
    var displayName: String {
        switch self {
        case .success: return "Success"
        case .noFill: return "No Fill"
        case .rateLimitExceeded: return "429 Rate Limit"
        case .serverError: return "503 Server Error"
        case .timeout: return "Timeout"
        }
    }
}

// MARK: - MockURLProtocol

class MockURLProtocol: URLProtocol {
    static var currentScenario: DemoScenario = .success
    
    override class func canInit(with request: URLRequest) -> Bool {
        // Intercept all HTTP/HTTPS requests
        return request.url?.scheme == "https" || request.url?.scheme == "http"
    }
    
    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        return request
    }
    
    override func startLoading() {
        let scenario = MockURLProtocol.currentScenario
        
        switch scenario {
        case .success:
            sendSuccessResponse()
        case .noFill:
            sendNoFillResponse()
        case .rateLimitExceeded:
            sendRateLimitResponse()
        case .serverError:
            sendServerErrorResponse()
        case .timeout:
            sendTimeoutResponse()
        }
    }
    
    override func stopLoading() {
        // Cleanup if needed
    }
    
    private func sendSuccessResponse() {
        let mockAd = """
        {
            "ad_id": "mock_ad_123",
            "placement": "test_placement",
            "ad_type": "interstitial",
            "creative": {"banner": {"url": "https://example.com/ad.jpg", "width": 320, "height": 480}},
            "network_name": "MockNetwork",
            "cpm": 5.50,
            "expires_at": "\(futureTimestamp())",
            "metadata": {}
        }
        """
        
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: 200,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": "application/json"]
        )!
        
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: mockAd.data(using: .utf8)!)
        client?.urlProtocolDidFinishLoading(self)
    }
    
    private func sendNoFillResponse() {
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: 204,
            httpVersion: "HTTP/1.1",
            headerFields: nil
        )!
        
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocolDidFinishLoading(self)
    }
    
    private func sendRateLimitResponse() {
        let errorBody = """
        {"error": "Rate limit exceeded", "retry_after": 60}
        """
        
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: 429,
            httpVersion: "HTTP/1.1",
            headerFields: ["Retry-After": "60"]
        )!
        
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: errorBody.data(using: .utf8)!)
        client?.urlProtocolDidFinishLoading(self)
    }
    
    private func sendServerErrorResponse() {
        let errorBody = """
        {"error": "Service unavailable"}
        """
        
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: 503,
            httpVersion: "HTTP/1.1",
            headerFields: nil
        )!
        
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: errorBody.data(using: .utf8)!)
        client?.urlProtocolDidFinishLoading(self)
    }
    
    private func sendTimeoutResponse() {
        // Simulate timeout by not responding
        DispatchQueue.global().asyncAfter(deadline: .now() + 15) {
            let error = NSError(
                domain: NSURLErrorDomain,
                code: NSURLErrorTimedOut,
                userInfo: [NSLocalizedDescriptionKey: "Request timed out"]
            )
            self.client?.urlProtocol(self, didFailWithError: error)
        }
    }
    
    private func futureTimestamp() -> String {
        let future = Date().addingTimeInterval(3600) // 1 hour from now
        let formatter = ISO8601DateFormatter()
        return formatter.string(from: future)
    }
}
