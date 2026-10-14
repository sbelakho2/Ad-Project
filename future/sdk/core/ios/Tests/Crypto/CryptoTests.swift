import XCTest
import Foundation
import Crypto
@testable import RivalApexMediationSDK

/// Cryptographic workflow tests: HMAC, SHA256, AES-GCM, nonces, constant-time
/// comparison, and the Ed25519 config-verification path end to end.
final class CryptoTests: XCTestCase {

    private extension Data {
        var hexString: String {
            map { String(format: "%02x", $0) }.joined()
        }
    }

    func testHMACSHA256Signing() throws {
        let message = "test-message-payload"
        let key = "secret-key-12345"

        let messageData = message.data(using: .utf8)!
        let keyData = SymmetricKey(data: key.data(using: .utf8)!)

        let signature = HMAC<SHA256>.authenticationCode(for: messageData, using: keyData)
        let signatureHex = Data(signature).hexString

        XCTAssertFalse(signatureHex.isEmpty)
        XCTAssertEqual(signatureHex.count, 64)
    }

    func testHMACSignatureVerification() throws {
        let message = "authenticated-request"
        let key = "shared-secret"
        let messageData = message.data(using: .utf8)!
        let keyData = SymmetricKey(data: key.data(using: .utf8)!)

        let signature = HMAC<SHA256>.authenticationCode(for: messageData, using: keyData)
        XCTAssertTrue(
            HMAC<SHA256>.isValidAuthenticationCode(signature, authenticating: messageData, using: keyData)
        )

        let tamperedMessage = "tampered-request".data(using: .utf8)!
        XCTAssertFalse(
            HMAC<SHA256>.isValidAuthenticationCode(signature, authenticating: tamperedMessage, using: keyData)
        )
    }

    func testSHA256Hashing() throws {
        let inputData = "test-input-data".data(using: .utf8)!

        let hashHex = SHA256.hash(data: inputData).compactMap { String(format: "%02x", $0) }.joined()
        XCTAssertEqual(hashHex.count, 64)

        let hashHex2 = SHA256.hash(data: inputData).compactMap { String(format: "%02x", $0) }.joined()
        XCTAssertEqual(hashHex, hashHex2)
    }

    func testAESGCMEncryptionDecryption() throws {
        let plaintext = "sensitive-user-data"
        let plaintextData = plaintext.data(using: .utf8)!
        let key = SymmetricKey(size: .bits256)

        let sealedBox = try AES.GCM.seal(plaintextData, using: key)
        XCTAssertNotNil(sealedBox.ciphertext)
        XCTAssertNotEqual(sealedBox.ciphertext, plaintextData)

        let decryptedData = try AES.GCM.open(sealedBox, using: key)
        XCTAssertEqual(String(data: decryptedData, encoding: .utf8), plaintext)
    }

    func testRandomNonceGeneration() throws {
        let nonce1 = AES.GCM.Nonce()
        let nonce2 = AES.GCM.Nonce()
        XCTAssertNotEqual(Data(nonce1), Data(nonce2))
    }

    func testTimingSafeComparison() {
        let data1 = "correct-secret".data(using: .utf8)!
        let data2 = "correct-secret".data(using: .utf8)!
        let data3 = "wrong-secret".data(using: .utf8)!

        func constantTimeCompare(_ lhs: Data, _ rhs: Data) -> Bool {
            guard lhs.count == rhs.count else { return false }
            return lhs.withUnsafeBytes { lhsBytes in
                rhs.withUnsafeBytes { rhsBytes in
                    var result = 0
                    for i in 0..<lhs.count {
                        result |= Int(lhsBytes[i]) ^ Int(rhsBytes[i])
                    }
                    return result == 0
                }
            }
        }

        XCTAssertTrue(constantTimeCompare(data1, data2))
        XCTAssertFalse(constantTimeCompare(data1, data3))
    }

    func testEd25519ConfigVerificationRoundTrip() throws {
        let keyPair = Curve25519.Signing.PrivateKey()
        let verifier = SignatureVerifier(
            testMode: false,
            productionPublicKey: keyPair.publicKey.rawRepresentation.hexString
        )
        let message = SignatureVerifier.createCanonicalMessage(
            configId: "cfg_crypto_1",
            version: 7,
            timestamp: 1_700_000_000
        )
        let signature = try keyPair.signature(for: Data(message.utf8)).base64EncodedString()

        XCTAssertTrue(try verifier.verifySignature(message: message, signatureBase64: signature))
    }

    func testEd25519ConfigVerificationRejectsTampering() throws {
        let keyPair = Curve25519.Signing.PrivateKey()
        let verifier = SignatureVerifier(
            testMode: false,
            productionPublicKey: keyPair.publicKey.rawRepresentation.hexString
        )
        let message = SignatureVerifier.createCanonicalMessage(
            configId: "cfg_crypto_2",
            version: 1,
            timestamp: 1_700_000_000
        )
        let signature = try keyPair.signature(for: Data(message.utf8)).base64EncodedString()

        XCTAssertThrowsError(
            try verifier.verifySignature(message: message + " ", signatureBase64: signature)
        ) { error in
            XCTAssertEqual(error as? SignatureError, .verificationFailed)
        }
    }
}
