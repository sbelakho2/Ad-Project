import Foundation

/// Lightweight HTTP ad loader used when `SDKConfig.testMode` is enabled.
/// Sends auction-style requests through the standard URLSession pipeline so
/// tests can deterministically intercept traffic via `NetworkTestHooks`.
final class TestModeAdLoader: @unchecked Sendable {
    private let requestURL: URL
    private let session: URLSession
    private let decoder: JSONDecoder

    init?(auctionEndpoint: String, requestTimeout: TimeInterval = 2.0) {
        guard let url = Self.normalize(endpoint: auctionEndpoint) else {
            return nil
        }
        requestURL = url

        let configuration = URLSessionConfiguration.apexDefault(
            requestTimeout: requestTimeout,
            resourceTimeout: requestTimeout * 2
        )
        session = URLSession(configuration: configuration)

        decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .iso8601
    }

    func loadAd(for placement: PlacementConfig) async throws -> Ad {
        var request = URLRequest(url: requestURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let payload: [String: Any] = [
            "placement_id": placement.placementId,
            "ad_type": placement.adType.rawValue,
            "floor_cpm": placement.floorCPM
        ]
        request.httpBody = try JSONSerialization.data(withJSONObject: payload)

        do {
            let (data, response) = try await session.apexData(for: request)
            guard let httpResponse = response as? HTTPURLResponse else {
                throw SDKError.networkError(underlying: "Invalid response type")
            }

            switch httpResponse.statusCode {
            case 200:
                let envelope = try decoder.decode(TestModeAdEnvelope.self, from: data)
                return envelope.makeAd(fallbackPlacement: placement)
            case 204:
                throw SDKError.noFill
            case 429:
                throw SDKError.status_429(message: Self.message(from: data) ?? "Rate limit exceeded")
            case 500...599:
                throw SDKError.status_5xx(
                    code: httpResponse.statusCode,
                    message: Self.message(from: data) ?? "Server error"
                )
            default:
                throw SDKError.fromHTTPStatus(
                    code: httpResponse.statusCode,
                    message: Self.message(from: data)
                )
            }
        } catch let error as SDKError {
            throw error
        } catch let error as URLError {
            if error.code == .timedOut {
                throw SDKError.timeout
            }
            throw SDKError.networkError(underlying: error.localizedDescription)
        } catch {
            throw SDKError.internalError(message: error.localizedDescription)
        }
    }

    func shutdown() {
        session.invalidateAndCancel()
    }

    private static func normalize(endpoint: String) -> URL? {
        guard var url = URL(string: endpoint) else {
            return nil
        }

        if url.path.isEmpty || url.path == "/" {
            url.appendPathComponent("v1")
            url.appendPathComponent("auction")
            return url
        }

        return url
    }

    private static func message(from data: Data) -> String? {
        guard
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        if let errorMessage = object["error"] as? String {
            return errorMessage
        }
        if let message = object["message"] as? String {
            return message
        }
        return nil
    }
}

private struct TestModeAdEnvelope: Decodable {
    let adId: String?
    let placement: String?
    let adType: AdType?
    let creative: CreativePayload?
    let networkName: String?
    let cpm: Double?
    let expiresAt: Date?
    let metadata: [String: String]?

    func makeAd(fallbackPlacement: PlacementConfig) -> Ad {
        let resolvedAdType = Self.resolveAdType(preferred: adType, fallback: fallbackPlacement.adType)
        let creativeAsset = creative?.toCreative(preferredType: resolvedAdType)
            ?? Self.defaultCreative(for: resolvedAdType)
        return Ad(
            adId: adId ?? UUID().uuidString,
            placement: fallbackPlacement.placementId,
            adType: resolvedAdType,
            creative: creativeAsset,
            networkName: networkName ?? "MockNetwork",
            cpm: cpm ?? max(fallbackPlacement.floorCPM, 0.01),
            expiresAt: expiresAt ?? Date().addingTimeInterval(3600),
            metadata: metadata ?? [:]
        )
    }

    private static func resolveAdType(preferred: AdType?, fallback: AdType) -> AdType {
        guard let preferred else { return fallback }
        return preferred == fallback ? preferred : fallback
    }

    private static func defaultCreative(for adType: AdType) -> Creative {
        switch adType {
        case .banner:
            return .banner(
                imageURL: "https://example.com/mock-banner.png",
                clickURL: "https://example.com/click",
                width: 320,
                height: 50
            )
        case .rewarded, .rewardedInterstitial, .interstitial, .appOpen:
            return .video(
                videoURL: "https://example.com/mock-video.mp4",
                clickURL: "https://example.com/click",
                duration: 30
            )
        case .native:
            return .native(
                title: "Test Ad",
                description: "Mock creative",
                iconURL: "https://example.com/icon.png",
                imageURL: "https://example.com/image.png",
                clickURL: "https://example.com/click",
                ctaText: "Install"
            )
        }
    }
}

private struct CreativePayload: Decodable {
    let banner: BannerPayload?
    let video: VideoPayload?
    let native: NativePayload?

    func toCreative(preferredType: AdType) -> Creative? {
        if let banner {
            return .banner(
                imageURL: banner.url,
                clickURL: banner.clickUrl ?? banner.url,
                width: banner.width ?? 320,
                height: banner.height ?? 50
            )
        }
        if let video {
            return .video(
                videoURL: video.url,
                clickURL: video.clickUrl ?? video.url,
                duration: video.duration ?? 30
            )
        }
        if let native {
            return .native(
                title: native.title ?? "Test Ad",
                description: native.description ?? "Mock creative",
                iconURL: native.iconUrl ?? "https://example.com/icon.png",
                imageURL: native.imageUrl ?? "https://example.com/image.png",
                clickURL: native.clickUrl ?? "https://example.com/click",
                ctaText: native.ctaText ?? "Install"
            )
        }
        return nil
    }
}

private struct BannerPayload: Decodable {
    let url: String
    let clickUrl: String?
    let width: Int?
    let height: Int?

    enum CodingKeys: String, CodingKey {
        case url
        case clickUrl = "click_url"
        case width
        case height
    }
}

private struct VideoPayload: Decodable {
    let url: String
    let clickUrl: String?
    let duration: Int?

    enum CodingKeys: String, CodingKey {
        case url
        case clickUrl = "click_url"
        case duration
    }
}

private struct NativePayload: Decodable {
    let title: String?
    let description: String?
    let iconUrl: String?
    let imageUrl: String?
    let clickUrl: String?
    let ctaText: String?

    enum CodingKeys: String, CodingKey {
        case title
        case description
        case iconUrl = "icon_url"
        case imageUrl = "image_url"
        case clickUrl = "click_url"
        case ctaText = "cta_text"
    }
}
