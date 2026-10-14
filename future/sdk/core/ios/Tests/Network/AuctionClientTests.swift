import XCTest
@testable import RivalApexMediationSDK

final class AuctionClientTests: XCTestCase {
    private var session: URLSession!
    private var client: AuctionClient!
    private let baseURL = "https://test.example.com"

    private var requestURL: URL {
        URL(string: "https://test.example.com/v1/auction")!
    }

    override func setUp() {
        super.setUp()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [AuctionClientProtocolStub.self]
        session = URLSession(configuration: config)
        AuctionClientProtocolStub.reset()
        client = AuctionClient(baseURL: baseURL, apiKey: "key", timeout: 1.0, session: session)
    }

    override func tearDown() {
        client = nil
        session.invalidateAndCancel()
        session = nil
        AuctionClientProtocolStub.reset()
        super.tearDown()
    }

    func testNoFill204MapsToSDKError() async {
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            let response = HTTPURLResponse(url: requestURL, statusCode: 204, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "interstitial")
            XCTFail("expected no-fill error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .noFill)
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    func testHTTP429MapsToStatus429() async {
        let payload = Data("Rate limited".utf8)
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            let response = HTTPURLResponse(url: requestURL, statusCode: 429, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, payload)
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "rewarded")
            XCTFail("expected 429 error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .status_429(message: "Rate limited"))
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    func testHTTP503MapsToServerError() async {
        let payload = Data("oops".utf8)
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            let response = HTTPURLResponse(url: requestURL, statusCode: 503, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, payload)
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "rewarded")
            XCTFail("expected server error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .status_5xx(code: 503, message: "oops"))
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    func testTimeoutMapsToTimeoutError() async {
        AuctionClientProtocolStub.handler = { _ in
            throw URLError(.timedOut)
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "rewarded")
            XCTFail("expected timeout error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .timeout)
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    func testDnsFailureMapsToNetworkError() async {
        AuctionClientProtocolStub.handler = { _ in
            throw URLError(.cannotFindHost)
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "banner")
            XCTFail("expected network error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .networkError(underlying: URLError(.cannotFindHost).localizedDescription))
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    func testCaptivePortalRedirectMapsToNetworkError() async {
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            let response = HTTPURLResponse(url: requestURL, statusCode: 302, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "banner")
            XCTFail("expected redirect error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .networkError(underlying: "HTTP 302"))
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }
}

final class AuctionClientProtocolStub: URLProtocol {
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = Self.handler else {
            fatalError("handler not set")
        }
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

    static func reset() {
        handler = nil
    }
}
