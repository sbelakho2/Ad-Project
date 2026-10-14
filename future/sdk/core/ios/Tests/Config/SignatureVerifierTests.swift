import XCTest

#if canImport(CryptoKit)
import CryptoKit

@testable import RivalApexMediationSDK

final class SignatureVerifierTests: XCTestCase {
    func testVerifySignatureWithGeneratedKey() throws {
        let keyPair = Curve25519.Signing.PrivateKey()
        let publicKeyHex = keyPair.publicKey.rawRepresentation.hexString
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: publicKeyHex)

        let message = "config_123"
        let signatureData = try keyPair.signature(for: Data(message.utf8))
        let signatureBase64 = signatureData.base64EncodedString()

        XCTAssertTrue(try verifier.verifySignature(message: message, signatureBase64: signatureBase64))
    }

    func testVerifySignatureRejectsTampering() throws {
        let keyPair = Curve25519.Signing.PrivateKey()
        let publicKeyHex = keyPair.publicKey.rawRepresentation.hexString
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: publicKeyHex)

        let message = "config_123"
        let signatureBase64 = try keyPair.signature(for: Data(message.utf8)).base64EncodedString()

        XCTAssertThrowsError(
            try verifier.verifySignature(message: message + "_tampered", signatureBase64: signatureBase64)
        ) { error in
            XCTAssertEqual(error as? SignatureError, .verificationFailed)
        }
    }

    func testVerifySignatureRequiresPublicKeyInProduction() {
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: nil)

        XCTAssertThrowsError(try verifier.verifySignature(message: "msg", signatureBase64: "AAAA")) { error in
            XCTAssertEqual(error as? SignatureError, .noPublicKey)
        }
    }

    func testVerifyEd25519RawMatchesHighLevelAPI() throws {
        let keyPair = Curve25519.Signing.PrivateKey()
        let publicKeyHex = keyPair.publicKey.rawRepresentation.hexString
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: publicKeyHex)

        let payload = Data("payload".utf8)
        let signature = try keyPair.signature(for: payload)

        XCTAssertTrue(try verifier.verifyEd25519Raw(message: payload, signature: signature, publicKeyHex: publicKeyHex))
    }

    func testCanonicalMessageIsDeterministic() {
        let generated = SignatureVerifier.createCanonicalMessage(configId: "config_1", version: 4, timestamp: 1234)
        let regenerated = SignatureVerifier.createCanonicalMessage(configId: "config_1", version: 4, timestamp: 1234)

        XCTAssertEqual(generated, regenerated)
        XCTAssertEqual(generated, "{\"config_id\":\"config_1\",\"timestamp\":1234,\"version\":4}")
    }

    func testMalformedBase64Throws() {
        let verifier = SignatureVerifier(testMode: true, productionPublicKey: nil)

        XCTAssertThrowsError(try verifier.verifySignature(message: "msg", signatureBase64: "!not_base64!")) { error in
            XCTAssertEqual(error as? SignatureError, .malformedSignature("Invalid Base64 encoding"))
        }
    }

    // MARK: - Section 3 Enhanced Coverage Tests

    func testTestModeBypassWithDevKey() throws {
        // In test mode, should use dev public key even without production key
        let verifier = SignatureVerifier(testMode: true, productionPublicKey: nil)
        
        let message = "test message for dev key"
        let testSignature = Data(repeating: 0, count: 64).base64EncodedString()
        
        // Should not throw noPublicKey error in test mode
        do {
            _ = try verifier.verifySignature(message: message, signatureBase64: testSignature)
        } catch SignatureError.noPublicKey {
            XCTFail("Test mode should not require production public key")
        } catch {
            // Other errors (like verificationFailed) are acceptable
        }
    }

    func testInvalidSignatureLengthThrows() throws {
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
        let message = "test message"
        
        // Create signature with wrong length (32 bytes instead of 64)
        let shortSignature = Data(repeating: 0, count: 32).base64EncodedString()
        
        XCTAssertThrowsError(try verifier.verifySignature(message: message, signatureBase64: shortSignature)) { error in
            if case SignatureError.invalidSignatureLength? = error as? SignatureError {
                // expected
            } else {
                XCTFail("Expected invalidSignatureLength error, got \(error)")
            }
        }
    }

    func testMalformedPublicKeyHexThrows() throws {
        let message = "test"
        let signature = Data(repeating: 0, count: 64)
        let badPublicKey = "not-valid-hex-zzz"
        
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: badPublicKey)
        
        XCTAssertThrowsError(try verifier.verifyEd25519Raw(message: message.data(using: .utf8)!, signature: signature, publicKeyHex: badPublicKey)) { error in
            if case SignatureError.malformedPublicKey? = error as? SignatureError {
                // expected
            } else {
                XCTFail("Expected malformedPublicKey error, got \(error)")
            }
        }
    }

    func testEmptySignatureThrows() throws {
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
        let message = "test message"
        let emptySignature = ""
        
        XCTAssertThrowsError(try verifier.verifySignature(message: message, signatureBase64: emptySignature)) { error in
            if case SignatureError.invalidSignatureLength(let length)? = error as? SignatureError {
                XCTAssertEqual(length, 0)
            } else {
                XCTFail("Expected invalidSignatureLength error for empty string, got \(error)")
            }
        }
    }

    func testCorrectLengthButInvalidContentFails() throws {
        let privateKey = Curve25519.Signing.PrivateKey()
        let publicKeyHex = privateKey.publicKey.rawRepresentation.hexString
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: publicKeyHex)
        
        let message = "test message"
        // Create 64 bytes of garbage data (correct length but not a valid signature)
        let invalidSignature = Data(repeating: 0xFF, count: 64).base64EncodedString()
        
        XCTAssertThrowsError(try verifier.verifySignature(message: message, signatureBase64: invalidSignature)) { error in
            if case SignatureError.verificationFailed? = error as? SignatureError {
                // expected
            } else {
                XCTFail("Expected verificationFailed error, got \(error)")
            }
        }
    }

    func testProductionModeWithoutKeyThrows() throws {
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: nil)
        let message = "test"
        let signature = Data(repeating: 0, count: 64).base64EncodedString()
        
        XCTAssertThrowsError(try verifier.verifySignature(message: message, signatureBase64: signature)) { error in
            XCTAssertEqual(error as? SignatureError, .noPublicKey)
        }
    }

    func testValidSignatureWithWrongKeyFails() throws {
        // Generate key pair A
        let keyA = Curve25519.Signing.PrivateKey()
        _ = keyA.publicKey.rawRepresentation.hexString
        
        // Generate key pair B
        let keyB = Curve25519.Signing.PrivateKey()
        let publicKeyBHex = keyB.publicKey.rawRepresentation.hexString
        
        let message = "test message"
        let messageData = message.data(using: .utf8)!
        
        // Sign with key A
        let signatureA = try keyA.signature(for: messageData)
        let signatureABase64 = signatureA.base64EncodedString()
        
        // Verify with key B (should fail)
        let verifier = SignatureVerifier(testMode: false, productionPublicKey: publicKeyBHex)
        
        XCTAssertThrowsError(try verifier.verifySignature(message: message, signatureBase64: signatureABase64)) { error in
            if case SignatureError.verificationFailed? = error as? SignatureError {
                // expected - signature is valid format but signed by wrong key
            } else {
                XCTFail("Expected verificationFailed error, got \(error)")
            }
        }
    }
}

private extension Data {
    var hexString: String {
        map { String(format: "%02x", $0) }.joined()
    }
}

#else

final class SignatureVerifierTests: XCTestCase {
    func testCryptoKitUnavailableOnLinux() throws {
        throw XCTSkip("CryptoKit not available on this platform")
    }
}

#endif
