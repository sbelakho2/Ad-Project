import Foundation

public enum IntegrationValidator {
    public struct Result: Equatable {
        public let errors: [String]
        public let warnings: [String]
        public let info: [String]

        public var isValid: Bool { errors.isEmpty }

        public func hasErrors() -> Bool { !errors.isEmpty }

        public func hasWarnings() -> Bool { !warnings.isEmpty }
    }

    public static func quickValidate(config: SDKConfig) -> Result {
        var errors: [String] = []
        var warnings: [String] = []
        var info: [String] = []

        if config.appId.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines).isEmpty {
            errors.append("App ID must not be empty")
        }

        if URL(string: config.configEndpoint)?.scheme == nil {
            errors.append("Config endpoint is invalid")
        }

        if URL(string: config.auctionEndpoint)?.scheme == nil {
            errors.append("Auction endpoint is invalid")
        }

        if config.testMode {
            warnings.append("Test mode enabled; disable before release")
        }

        if errors.isEmpty {
            info.append("Configuration looks valid")
        }

        return Result(errors: errors, warnings: warnings, info: info)
    }

    public static func validate(config: SDKConfig) async -> Result {
        let quick = quickValidate(config: config)
        var info = quick.info
        info.append("Async validation complete")
        return Result(errors: quick.errors, warnings: quick.warnings, info: info)
    }
}
