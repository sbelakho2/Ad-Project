import Foundation

#if canImport(CryptoKit)
import CryptoKit
#elseif canImport(Crypto)
import Crypto
#else
#error("CryptoKit or SwiftCrypto is required for signature verification")
#endif

/// Errors thrown during signature verification operations.
public enum SignatureError: Error, Equatable, LocalizedError {
    case malformedSignature(String)
    case invalidSignatureLength(Int)
    case noPublicKey
    case malformedPublicKey
    case invalidPublicKey
    case verificationFailed

    public var errorDescription: String? {
        switch self {
        case .malformedSignature(let reason):
            return "Malformed signature: \(reason)"
        case .invalidSignatureLength(let length):
            return "Invalid signature length: \(length) bytes (expected 64)"
        case .noPublicKey:
            return "No public key configured"
        case .malformedPublicKey:
            return "Public key is not valid hex"
        case .invalidPublicKey:
            return "Could not create Ed25519 public key from data"
        case .verificationFailed:
            return "Signature verification failed"
        }
    }
}

/// Utility responsible for verifying Ed25519 signatures used for OTA config updates.
public final class SignatureVerifier {
    private static let devTestPublicKeyHex = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    private static let signatureLength = 64

    private let testMode: Bool
    private let productionPublicKey: String?

    public init(testMode: Bool, productionPublicKey: String?) {
        self.testMode = testMode
        self.productionPublicKey = productionPublicKey
    }

    /// Verify a Base64-encoded Ed25519 signature for the supplied message.
    @discardableResult
    public func verifySignature(message: String, signatureBase64: String) throws -> Bool {
        let publicKeyHex = try resolvePublicKey()
        
        guard let signatureData = Data(base64Encoded: signatureBase64) else {
            throw SignatureError.malformedSignature("Invalid Base64 encoding")
        }

        guard signatureData.count == Self.signatureLength else {
            throw SignatureError.invalidSignatureLength(signatureData.count)
        }

        let messageData = Data(message.utf8)

        let isValid = try verifyEd25519Raw(message: messageData, signature: signatureData, publicKeyHex: publicKeyHex)
        guard isValid else { throw SignatureError.verificationFailed }
        return true
    }

    /// Produce a deterministic canonical message representation for signing.
    public static func createCanonicalMessage(configId: String, version: Int, timestamp: Int64) -> String {
        "{\"config_id\":\"\(configId)\",\"timestamp\":\(timestamp),\"version\":\(version)}"
    }

    /// Validate Ed25519 signature bytes against the supplied public key.
    public func verifyEd25519Raw(message: Data, signature: Data, publicKeyHex: String) throws -> Bool {
        guard signature.count == Self.signatureLength else {
            throw SignatureError.invalidSignatureLength(signature.count)
        }

        guard let publicKeyData = Data(hexString: publicKeyHex) else {
            throw SignatureError.malformedPublicKey
        }

        let publicKey: Curve25519.Signing.PublicKey
        do {
            publicKey = try Curve25519.Signing.PublicKey(rawRepresentation: publicKeyData)
        } catch {
            throw SignatureError.invalidPublicKey
        }

        return publicKey.isValidSignature(signature, for: message)
    }

    private func resolvePublicKey() throws -> String {
        if let productionKey = productionPublicKey, !productionKey.isEmpty {
            return productionKey
        }

        if testMode {
            return Self.devTestPublicKeyHex
        }

        throw SignatureError.noPublicKey
    }
}

// MARK: - Data helpers

private extension Data {
    init?(hexString: String) {
        let cleaned = hexString.trimmingCharacters(in: .whitespacesAndNewlines)
        guard cleaned.count % 2 == 0 else { return nil }

        var data = Data(capacity: cleaned.count / 2)
        var index = cleaned.startIndex

        while index < cleaned.endIndex {
            let nextIndex = cleaned.index(index, offsetBy: 2)
            guard nextIndex <= cleaned.endIndex else { return nil }
            let byteString = cleaned[index..<nextIndex]
            guard let byte = UInt8(byteString, radix: 16) else { return nil }
            data.append(byte)
            index = nextIndex
        }

        self = data
    }

    var hexString: String {
        map { String(format: "%02x", $0) }.joined()
    }
}
