import Foundation

/// Minimal IAB TCF v2 parser to extract a subset of consent signals for gating
/// personalization. Reads the core segment only and extracts Purpose 1
/// ("store and/or access information on a device") from the purposesConsent
/// bitfield. A missing, malformed, or truncated string yields fail-closed
/// defaults. Mirrors the Android `TCFParser`.
public enum TCFParser {
    public struct Core: Equatable, Sendable {
        public let gdprApplies: Bool
        public let purpose1Consent: Bool
        public let raw: String
    }

    // version(6) created(36) lastUpdated(36) cmpId(12) cmpVersion(12)
    // consentScreen(6) consentLanguage(12) vendorListVersion(12)
    // tcfPolicyVersion(6) isServiceSpecific(1) useNonStandardStacks(1)
    // specialFeatureOptIns(12) then purposesConsent(24).
    private static let purposesConsentOffset = 152

    public static func parse(tcfV2: String?, gdprApplies: Bool) -> Core {
        guard let tcfV2, !tcfV2.isEmpty else {
            return Core(gdprApplies: gdprApplies, purpose1Consent: false, raw: tcfV2 ?? "")
        }
        let coreSegment = String(tcfV2.prefix(while: { $0 != "." }))
        guard let bytes = decodeBase64URL(coreSegment), !bytes.isEmpty else {
            return Core(gdprApplies: gdprApplies, purpose1Consent: false, raw: tcfV2)
        }
        var reader = BitReader(bytes: [UInt8](bytes))
        guard reader.skip(purposesConsentOffset), let purposes = reader.read(24) else {
            return Core(gdprApplies: gdprApplies, purpose1Consent: false, raw: tcfV2)
        }
        // Purpose 1 is bit 0, the most significant bit of the 24-bit field.
        let purpose1 = (purposes >> 23) & 1 == 1
        return Core(gdprApplies: gdprApplies, purpose1Consent: purpose1, raw: tcfV2)
    }

    /// Dependency-free base64url decoding (RFC 4648 section 5, padding optional).
    static func decodeBase64URL(_ encoded: String) -> Data? {
        guard !encoded.isEmpty else { return nil }
        var padded = encoded
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let remainder = padded.count % 4
        if remainder > 0 {
            padded += String(repeating: "=", count: 4 - remainder)
        }
        guard let data = Data(base64Encoded: padded, options: [.ignoreUnknownCharacters]),
              !data.isEmpty
        else {
            return nil
        }
        return data
    }

    private struct BitReader {
        let bytes: [UInt8]
        var pos = 0

        var remaining: Int { bytes.count * 8 - pos }

        mutating func skip(_ count: Int) -> Bool {
            guard remaining >= count else { return false }
            pos += count
            return true
        }

        mutating func read(_ count: Int) -> Int? {
            guard count <= 32, remaining >= count else { return nil }
            var value = 0
            for _ in 0..<count {
                let byte = bytes[pos / 8]
                value = (value << 1) | Int((byte >> (7 - pos % 8)) & 1)
                pos += 1
            }
            return value
        }
    }
}
