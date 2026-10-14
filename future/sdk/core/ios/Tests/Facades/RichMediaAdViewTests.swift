#if canImport(UIKit) && canImport(WebKit)
import XCTest
@testable import RivalApexMediationSDK

@MainActor
final class RichMediaAdViewTests: XCTestCase {
    func testMemoryWarningClearsContent() async {
        let view = RichMediaAdView(frame: .init(x: 0, y: 0, width: 320, height: 50))
        view.webView?.loadHTMLString("<html><body>ad</body></html>", baseURL: nil)

        // Simulate memory warning
        NotificationCenter.default.post(name: UIApplication.didReceiveMemoryWarningNotification, object: nil)

        // Allow the handler Task to run
        try? await Task.sleep(nanoseconds: 50_000_000)

        let urlString = view.webView?.url?.absoluteString
        XCTAssertTrue(urlString == nil || urlString == "about:blank")
        withExtendedLifetime(view) {}
    }
}
#endif
