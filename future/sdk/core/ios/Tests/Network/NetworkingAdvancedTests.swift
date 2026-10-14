import XCTest
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
@testable import RivalApexMediationSDK

/// Tests for networking layer with custom URLProtocol mocks
class NetworkingAdvancedTests: XCTestCase {
    
    var session: URLSession!
    
    override func setUp() {
        super.setUp()
        // Configure URLSession with mock protocol
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        session = URLSession(configuration: config)
    }
    
    override func tearDown() {
        MockURLProtocol.reset()
        session = nil
        super.tearDown()
    }
    
    // MARK: - Network Request Tests
    
    func testHTTPRequestWithHeaders() async throws {
        let expectedURL = URL(string: "https://api.example.com/auction")!
        let expectedHeaders = ["Authorization": "Bearer test-token", "Content-Type": "application/json"]
        let responseBody = """
        {"status":"success","bidPrice":1.50}
        """.data(using: .utf8)!
        
        MockURLProtocol.mockResponse(url: expectedURL, statusCode: 200, headers: expectedHeaders, body: responseBody)
        
        var request = URLRequest(url: expectedURL)
        request.httpMethod = "POST"
        request.setValue("Bearer test-token", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        
        let (data, response) = try await session.apexData(for: request)
        
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        XCTAssertNotNil(data)
        XCTAssertGreaterThan(data.count, 0)
    }
    
    func testNetworkTimeout() async throws {
        let url = URL(string: "https://api.example.com/slow")!
        MockURLProtocol.mockDelay(url: url, delay: 5.0) // 5 second delay
        
        var request = URLRequest(url: url)
        request.timeoutInterval = 2.0 // 2 second timeout
        
        do {
            _ = try await session.apexData(for: request)
            XCTFail("Expected timeout error")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .timedOut)
        }
    }
    
    func testNetworkRetryOnFailure() async throws {
        let url = URL(string: "https://api.example.com/retry")!
        
        // First attempt fails with 503
        MockURLProtocol.mockResponse(url: url, statusCode: 503, body: Data())
        
        let request = URLRequest(url: url)
        let (_, response) = try await session.apexData(for: request)
        
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 503)
        
        // Retry should succeed (mock updated response)
        MockURLProtocol.mockResponse(url: url, statusCode: 200, body: Data())
        let (_, retryResponse) = try await session.apexData(for: request)
        XCTAssertEqual((retryResponse as? HTTPURLResponse)?.statusCode, 200)
    }
    
    func testNetworkErrorHandling() async throws {
        let url = URL(string: "https://api.example.com/error")!
        MockURLProtocol.mockError(url: url, error: URLError(.notConnectedToInternet))
        
        let request = URLRequest(url: url)
        
        do {
            _ = try await session.apexData(for: request)
            XCTFail("Expected network error")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .notConnectedToInternet)
        }
    }
}

// MARK: - Mock URLProtocol

class MockURLProtocol: URLProtocol {
    
    private static var mockResponses: [URL: (statusCode: Int, headers: [String: String]?, body: Data)] = [:]
    private static var mockErrors: [URL: Error] = [:]
    private static var mockDelays: [URL: TimeInterval] = [:]
    
    override class func canInit(with request: URLRequest) -> Bool {
        return true
    }
    
    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        return request
    }
    
    override func startLoading() {
        guard let url = request.url else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }
        
        // Check for mock error
        if let error = Self.mockErrors[url] {
            client?.urlProtocol(self, didFailWithError: error)
            return
        }
        
        // Check for mock delay
        let delay = Self.mockDelays[url] ?? 0
        let timeoutInterval = request.timeoutInterval
        let shouldTimeout = timeoutInterval > 0 && delay > timeoutInterval
        let dispatchDelay = shouldTimeout ? timeoutInterval : delay
        
        DispatchQueue.global().asyncAfter(deadline: .now() + dispatchDelay) { [weak self] in
            guard let self = self else { return }
            
            if shouldTimeout {
                self.client?.urlProtocol(self, didFailWithError: URLError(.timedOut))
                return
            }
            
            // Check for mock response
            if let mock = Self.mockResponses[url] {
                let response = HTTPURLResponse(
                    url: url,
                    statusCode: mock.statusCode,
                    httpVersion: "HTTP/1.1",
                    headerFields: mock.headers
                )!
                
                self.client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                self.client?.urlProtocol(self, didLoad: mock.body)
                self.client?.urlProtocolDidFinishLoading(self)
            } else {
                // Default 404
                let response = HTTPURLResponse(url: url, statusCode: 404, httpVersion: "HTTP/1.1", headerFields: nil)!
                self.client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                self.client?.urlProtocolDidFinishLoading(self)
            }
        }
    }
    
    override func stopLoading() {
        // Nothing to stop
    }
    
    // MARK: - Mocking Helpers
    
    static func mockResponse(url: URL, statusCode: Int, headers: [String: String]? = nil, body: Data) {
        mockResponses[url] = (statusCode, headers, body)
    }
    
    static func mockError(url: URL, error: Error) {
        mockErrors[url] = error
    }
    
    static func mockDelay(url: URL, delay: TimeInterval) {
        mockDelays[url] = delay
    }
    
    static func reset() {
        mockResponses.removeAll()
        mockErrors.removeAll()
        mockDelays.removeAll()
    }
}
