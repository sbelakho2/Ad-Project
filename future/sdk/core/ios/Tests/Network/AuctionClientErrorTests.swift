import XCTest
@testable import RivalApexMediationSDK

private final class MutableClock: ClockProtocol, @unchecked Sendable {
    var wallSeconds: TimeInterval
    var monotonicSecondsValue: TimeInterval

    init(wallSeconds: TimeInterval, monotonicSeconds: TimeInterval = 0) {
        self.wallSeconds = wallSeconds
        self.monotonicSecondsValue = monotonicSeconds
    }

    func now() -> Date { Date(timeIntervalSince1970: wallSeconds) }
    func nowMillis() -> Int64 { Int64(wallSeconds * 1000) }
    func monotonicMillis() -> Int64 { Int64(monotonicSecondsValue * 1000) }
    func monotonicSeconds() -> TimeInterval { monotonicSecondsValue }

    func advance(seconds: TimeInterval) {
        wallSeconds += seconds
        monotonicSecondsValue += seconds
    }
}

final class AuctionClientErrorTests: XCTestCase {
    private var session: URLSession!
    private var client: AuctionClient!
    private let baseURL = "https://test.example.com"
    private var recordedSleeps: [TimeInterval] = []
    private var previousClock: ClockProtocol!
    private var testClock: MutableClock!

    private var requestURL: URL {
        URL(string: "https://test.example.com/v1/auction")!
    }

    override func setUp() {
        super.setUp()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [AuctionClientProtocolStub.self]
        session = URLSession(configuration: config)
        AuctionClientProtocolStub.reset()
        recordedSleeps = []
        let seedSeconds = TimeInterval(1_735_000_000)
        previousClock = Clock.shared
        testClock = MutableClock(wallSeconds: seedSeconds, monotonicSeconds: 1000)
        Clock.shared = testClock
        client = makeClient()
    }

    override func tearDown() {
        client = nil
        session.invalidateAndCancel()
        session = nil
        AuctionClientProtocolStub.reset()
        recordedSleeps = []
        Clock.shared = previousClock
        super.tearDown()
    }

    func testServerErrorsRetryWithDeterministicBackoff() async {
        var attempts = 0
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            attempts += 1
            let response = HTTPURLResponse(url: requestURL, statusCode: 503, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, Data("server".utf8))
        }

        let retryConfig = AuctionClient.RetryConfiguration(maxAttempts: 3, initialBackoff: 0.25, maxBackoff: 0.5, backoffMultiplier: 2.0)
        client = makeClient(retryConfig: retryConfig)

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "rewarded")
            XCTFail("expected server failure after retries")
        } catch let error as SDKError {
            XCTAssertEqual(error, .status_5xx(code: 503, message: "server"))
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        XCTAssertEqual(attempts, 3)
        XCTAssertEqual(recordedSleeps.count, 2)
        XCTAssertEqual(recordedSleeps[0], 0.25, accuracy: 0.0001)
        XCTAssertEqual(recordedSleeps[1], 0.5, accuracy: 0.0001)
    }

    func testCircuitBreakerOpensAfterConsecutiveServerFailures() async {
        var failureAttempts = 0
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            failureAttempts += 1
            let response = HTTPURLResponse(url: requestURL, statusCode: 503, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, Data("server".utf8))
        }

        let retryConfig = AuctionClient.RetryConfiguration(maxAttempts: 1, initialBackoff: 0.1, maxBackoff: 0.1, backoffMultiplier: 1.0)
        let breakerConfig = AuctionClient.CircuitBreakerConfiguration(failureThreshold: 2, cooldown: 30)
        client = makeClient(retryConfig: retryConfig, circuitConfig: breakerConfig)

        for _ in 0..<2 {
            do {
                _ = try await client.requestBid(placementId: "p1", adType: "interstitial")
                XCTFail("expected server failure")
            } catch let error as SDKError {
                XCTAssertEqual(error, .status_5xx(code: 503, message: "server"))
            } catch {
                XCTFail("unexpected error: \(error)")
            }
        }
        XCTAssertEqual(failureAttempts, 2)

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "rewarded")
            XCTFail("expected circuit breaker error")
        } catch let error as SDKError {
            guard case .circuitBreakerOpen(let remaining) = error else {
                return XCTFail("expected circuit breaker error, got \(error)")
            }
            XCTAssertGreaterThan(remaining, 29.0)
        } catch {
            XCTFail("unexpected error: \(error)")
        }
        XCTAssertEqual(failureAttempts, 2)

        // Advance clock beyond cooldown and ensure requests reach the network again
        testClock.advance(seconds: 31)
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            let response = HTTPURLResponse(url: requestURL, statusCode: 204, httpVersion: "HTTP/1.1", headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "rewarded")
            XCTFail("expected no-fill response after cooldown")
        } catch let error as SDKError {
            XCTAssertEqual(error, .noFill)
        } catch {
            XCTFail("unexpected error after cooldown: \(error)")
        }
    }

    func testRateLimitErrorIncludesRetryAfterHeader() async {
        AuctionClientProtocolStub.handler = { [requestURL] _ in
            let headers = ["Retry-After": "5"]
            let response = HTTPURLResponse(url: requestURL, statusCode: 429, httpVersion: "HTTP/1.1", headerFields: headers)!
            return (response, Data("Rate limited".utf8))
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "banner")
            XCTFail("expected rate limit error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .status_429(message: "Rate limited (retry after 5s)"))
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    func testNavigationCancellationMapsToDeterministicError() async {
        AuctionClientProtocolStub.handler = { _ in
            throw URLError(.cancelled)
        }

        do {
            _ = try await client.requestBid(placementId: "p1", adType: "interstitial")
            XCTFail("expected cancellation error")
        } catch let error as SDKError {
            XCTAssertEqual(error, .networkError(underlying: "navigation_cancelled"))
        } catch {
            XCTFail("unexpected error: \(error)")
        }
    }

    private func makeClient(
        retryConfig: AuctionClient.RetryConfiguration = .standard,
        circuitConfig: AuctionClient.CircuitBreakerConfiguration = .standard
    ) -> AuctionClient {
        AuctionClient(
            baseURL: baseURL,
            apiKey: "key",
            timeout: 1.0,
            session: session,
            retryConfiguration: retryConfig,
            circuitBreakerConfiguration: circuitConfig,
            sleep: { [weak self] duration in
                guard let self else { return }
                try await self.recordSleep(duration)
            },
            dateProvider: { [weak self] in self?.testClock.now() ?? Date() },
            clock: testClock
        )
    }

    private func recordSleep(_ nanoseconds: UInt64) async throws {
        let seconds = TimeInterval(nanoseconds) / 1_000_000_000
        recordedSleeps.append(seconds)
    }
}
