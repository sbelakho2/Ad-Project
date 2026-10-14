import Foundation

public struct SDKConfig: Equatable {
    public let appId: String
    public let apiBaseUrl: String
    public let apiKey: String?
    public let testMode: Bool
    public let requestTimeoutMs: Int
    public let configPublicKeyPem: String?

    public init(appId: String,
                apiBaseUrl: String = "https://api.apexmediation.ee/api/v1",
                apiKey: String? = nil,
                testMode: Bool = false,
                requestTimeoutMs: Int = 5000,
                configPublicKeyPem: String? = nil) {
        self.appId = appId
        self.apiBaseUrl = apiBaseUrl
        self.apiKey = apiKey
        self.testMode = testMode
        self.requestTimeoutMs = requestTimeoutMs
        self.configPublicKeyPem = configPublicKeyPem
    }
}
