import XCTest
@testable import RivalApexMediationSDK

#if canImport(UIKit)
import UIKit

@MainActor
final class AdPresentationCoordinatorTests: XCTestCase {
    override func tearDown() {
        #if DEBUG
        AdPresentationCoordinator.shared.resetForTests()
        #endif
        super.tearDown()
    }

    func testBeginBlocksConcurrentPresentation() throws {
        let token = try AdPresentationCoordinator.shared.beginPresentation(placementId: "placement_a")
        XCTAssertThrowsError(try AdPresentationCoordinator.shared.beginPresentation(placementId: "placement_b")) { error in
            XCTAssertEqual(error as? SDKError, SDKError.presentationInProgress)
        }
        AdPresentationCoordinator.shared.finishPresentation(token)
    }

    func testFinishAllowsNewPresentation() throws {
        let token = try AdPresentationCoordinator.shared.beginPresentation(placementId: "placement_a")
        AdPresentationCoordinator.shared.finishPresentation(token)
        XCTAssertNoThrow(try AdPresentationCoordinator.shared.beginPresentation(placementId: "placement_b"))
    }
}
#else
final class AdPresentationCoordinatorTests: XCTestCase {
    func testUIKitUnavailable() throws {
        throw XCTSkip("UIKit not available on this platform")
    }
}
#endif
