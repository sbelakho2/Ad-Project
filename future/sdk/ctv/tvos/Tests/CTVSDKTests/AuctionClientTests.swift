import XCTest
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
@testable import CTVSDK

final class AuctionClientTests: XCTestCase {
    private var session: URLSession!
    private var config: SDKConfig!

    override func setUp() {
        super.setUp()
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [MockURLProtocol.self]
        session = URLSession(configuration: cfg)
        config = SDKConfig(appId: "app-1", apiBaseUrl: "https://api.apexmediation.ee/api/v1")
    }

    override func tearDown() {
        session.invalidateAndCancel()
        session = nil
        super.tearDown()
    }

    func testNoFill204() {
        MockURLProtocol.requestHandler = { request in
            let resp = HTTPURLResponse(url: request.url!, statusCode: 204, httpVersion: nil, headerFields: nil)!
            return (resp, Data())
        }
        let client = AuctionClient(config: config, session: session)
        let exp = expectation(description: "no_fill")
        client.requestBid(placementId: "p1", adFormat: "interstitial", consent: nil) { result in
            XCTAssertTrue(result.noFill)
            XCTAssertEqual(result.error, .noFill)
            exp.fulfill()
        }
        wait(for: [exp], timeout: 2.0)
    }

    func testParsesEnvelope200() {
        let obj: [String: Any] = [
            "response": [
                "requestId": "r1",
                "bidId": "b1",
                "adapter": "admob",
                "cpm": 1.23,
                "currency": "USD",
                "ttlSeconds": 300,
                "creativeUrl": "https://cdn.test/video.mp4",
                "tracking": ["impression": "http://i", "click": "http://c"],
                "payload": [:]
            ]
        ]
        let data = try! JSONSerialization.data(withJSONObject: obj)
        MockURLProtocol.requestHandler = { request in
            let resp = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type":"application/json"])!
            return (resp, data)
        }
        let client = AuctionClient(config: config, session: session)
        let exp = expectation(description: "ok")
        client.requestBid(placementId: "p1", adFormat: "interstitial", consent: nil) { result in
            XCTAssertFalse(result.noFill)
            XCTAssertNil(result.error)
            XCTAssertNotNil(result.win)
            exp.fulfill()
        }
        wait(for: [exp], timeout: 2.0)
    }
}

// MARK: - URLProtocol mock
final class MockURLProtocol: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?
    override class func canInit(with request: URLRequest) -> Bool { return true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { return request }
    override func startLoading() {
        guard let handler = MockURLProtocol.requestHandler else { fatalError("No handler set") }
        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }
    override func stopLoading() {}
}
