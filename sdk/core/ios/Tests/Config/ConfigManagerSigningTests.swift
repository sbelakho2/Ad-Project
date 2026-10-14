import XCTest
@testable import RivalApexMediationSDK

final class ConfigManagerSigningTests: XCTestCase {
    func testFailsClosedWhenVerifierMissingInProd() async {
        let cfg = SDKConfig(
            appId: "app",
            configEndpoint: "https://example.invalid",
            auctionEndpoint: "https://example.invalid",
            telemetryEnabled: true,
            logLevel: .info,
            testMode: false,
            configSignaturePublicKey: nil
        )

        let manager = ConfigManager(config: cfg, signatureVerifier: nil)

        do {
            _ = try await manager.loadConfig()
            XCTFail("expected missingSignature")
        } catch let cfgError as ConfigError {
            if case .missingSignature = cfgError {
                return
            }
            XCTFail("expected missingSignature, got \(cfgError)")
        } catch {
            XCTFail("expected ConfigError, got \(error)")
        }
    }
}

// MARK: - Canonical envelope pinning

extension ConfigManagerSigningTests {
    private func manager() -> ConfigManager {
        let cfg = SDKConfig(
            appId: "app",
            configEndpoint: "https://example.invalid",
            auctionEndpoint: "https://example.invalid",
            telemetryEnabled: true,
            logLevel: .info,
            testMode: false,
            configSignaturePublicKey: nil
        )
        return ConfigManager(config: cfg, signatureVerifier: nil)
    }

    func testSignaturePayloadPinsTheCrossPlatformByteOrder() throws {
        let remote = SDKRemoteConfig(
            version: 4,
            placements: [],
            adapters: [:],
            killswitches: [],
            telemetryEnabled: true,
            configId: "config_1",
            timestamp: 1234
        )
        XCTAssertEqual(
            try manager().createSignaturePayload(config: remote),
            "{\"config_id\":\"config_1\",\"timestamp\":1234,\"version\":4}"
        )
    }

    func testSignaturePayloadFailsClosedWithoutEnvelopeFields() {
        let legacy = SDKRemoteConfig(
            version: 4,
            placements: [],
            adapters: [:],
            killswitches: [],
            telemetryEnabled: true
        )
        XCTAssertThrowsError(try manager().createSignaturePayload(config: legacy))
    }
}
