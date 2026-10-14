import XCTest
@testable import RivalApexMediationSDK

final class IntegrationValidatorTests: XCTestCase {
    func testQuickValidateReturnsNoConfigurationErrorsForWellFormedConfig() {
        let result = IntegrationValidator.quickValidate(config: makeConfig())
        let configurationErrors = result.errors.filter { error in
            error.localizedCaseInsensitiveContains("app id") || error.localizedCaseInsensitiveContains("endpoint")
        }

        XCTAssertTrue(configurationErrors.isEmpty)
    }

    func testQuickValidateFlagsEmptyAppId() {
        let result = IntegrationValidator.quickValidate(config: makeConfig(appId: ""))

        XCTAssertTrue(result.errors.contains { $0.localizedCaseInsensitiveContains("app id") })
        XCTAssertFalse(result.isValid)
    }

    func testQuickValidateRejectsInvalidEndpoints() {
        let result = IntegrationValidator.quickValidate(
            config: makeConfig(configEndpoint: "not-a-url", auctionEndpoint: "also-not-a-url")
        )

        XCTAssertEqual(result.errors.filter { $0.localizedCaseInsensitiveContains("endpoint") }.count, 2)
    }

    func testQuickValidateWarnsWhenTestModeEnabled() {
        let result = IntegrationValidator.quickValidate(config: makeConfig(testMode: true))

        XCTAssertTrue(result.warnings.contains { $0.localizedCaseInsensitiveContains("test mode") })
    }

    func testValidateMatchesQuickValidateForErrors() async {
        let asyncResult = await IntegrationValidator.validate(config: makeConfig(testMode: true))
        let syncResult = IntegrationValidator.quickValidate(config: makeConfig(testMode: true))

        XCTAssertEqual(asyncResult.errors, syncResult.errors)
        XCTAssertTrue(asyncResult.hasWarnings())
        XCTAssertGreaterThanOrEqual(asyncResult.info.count, syncResult.info.count)
    }

    func testIsValidHelperReflectsErrorState() {
        let result = IntegrationValidator.quickValidate(config: makeConfig(appId: ""))

        XCTAssertTrue(result.hasErrors())
        XCTAssertFalse(result.isValid)
    }

    func testMultipleProblemsReportedTogether() {
        let result = IntegrationValidator.quickValidate(
            config: makeConfig(appId: "", configEndpoint: "invalid", auctionEndpoint: "invalid")
        )

        XCTAssertGreaterThanOrEqual(result.errors.count, 3)
    }

    private func makeConfig(
        appId: String = "app_12345678",
        configEndpoint: String = "https://config.example.com",
        auctionEndpoint: String = "https://auction.example.com",
        testMode: Bool = false
    ) -> SDKConfig {
        SDKConfig(
            appId: appId,
            configEndpoint: configEndpoint,
            auctionEndpoint: auctionEndpoint,
            telemetryEnabled: true,
            logLevel: .info,
            testMode: testMode
        )
    }
}
