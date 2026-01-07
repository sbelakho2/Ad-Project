import Foundation

/// Minimal IAB TCF v2 parser for CTV. Heuristic-only; preserves raw string.
///
/// This provides basic TCF v2 consent string validation and purpose extraction
/// for GDPR compliance. Full decoding requires the IAB TCF SDK, but this
/// heuristic parser handles common CTV use cases.
public enum TCFParser {
    
    /// Core consent information extracted from TCF v2 string
    public struct Core: Equatable {
        /// Whether GDPR applies to this user
        public let gdprApplies: Bool
        
        /// Purpose 1 consent (store/access info on device)
        public let purpose1Consent: Bool
        
        /// Raw TCF string for passthrough
        public let raw: String
        
        public init(gdprApplies: Bool, purpose1Consent: Bool, raw: String) {
            self.gdprApplies = gdprApplies
            self.purpose1Consent = purpose1Consent
            self.raw = raw
        }
    }
    
    /// Parse TCF v2 consent string
    ///
    /// - Parameters:
    ///   - tcfV2: The TCF v2 consent string (base64url encoded)
    ///   - gdprApplies: Whether GDPR applies to this user
    /// - Returns: Parsed Core consent information
    public static func parse(tcfV2: String?, gdprApplies: Bool) -> Core {
        guard let tcfV2 = tcfV2, !tcfV2.isEmpty else {
            return Core(gdprApplies: gdprApplies, purpose1Consent: false, raw: "")
        }
        
        // Basic validation: TCF v2 strings are base64url-encoded and contain dots
        // Format: <core-string>.<disclosed-vendors>.<allowed-vendors>.<publisher-tc>
        let looksValid = isValidTCFFormat(tcfV2)
        
        if looksValid {
            // Attempt to decode purpose 1 consent from the core segment
            let purpose1 = decodePurpose1Consent(tcfV2)
            return Core(gdprApplies: gdprApplies, purpose1Consent: purpose1, raw: tcfV2)
        }
        
        return Core(gdprApplies: gdprApplies, purpose1Consent: false, raw: tcfV2)
    }
    
    /// Check if string looks like valid TCF v2 format
    private static func isValidTCFFormat(_ string: String) -> Bool {
        // TCF v2 strings contain dots separating segments
        guard string.contains(".") else { return false }
        
        // All characters should be base64url safe (alphanumeric, -, _, .)
        let validCharacters = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_."))
        return string.unicodeScalars.allSatisfy { validCharacters.contains($0) }
    }
    
    /// Attempt to decode Purpose 1 consent from TCF string
    /// This is a heuristic - assumes consent if string appears valid
    private static func decodePurpose1Consent(_ tcfString: String) -> Bool {
        // Get core segment (before first dot)
        let segments = tcfString.split(separator: ".")
        guard let coreSegment = segments.first else { return false }
        
        // Decode base64url to bytes
        guard let data = base64URLDecode(String(coreSegment)) else { return false }
        
        // TCF v2 Core String structure (simplified):
        // - Version: 6 bits (should be 2)
        // - Created: 36 bits
        // - LastUpdated: 36 bits
        // - CmpId: 12 bits
        // - CmpVersion: 12 bits
        // - ConsentScreen: 6 bits
        // - ConsentLanguage: 12 bits
        // - VendorListVersion: 12 bits
        // - TcfPolicyVersion: 6 bits
        // - IsServiceSpecific: 1 bit
        // - UseNonStandardStacks: 1 bit
        // - SpecialFeatureOptIns: 12 bits
        // - PurposesConsent: 24 bits (bit 1 = Purpose 1)
        //
        // Total bits before PurposesConsent: 6+36+36+12+12+6+12+12+6+1+1+12 = 152 bits = 19 bytes
        // Purpose 1 is bit 153 (0-indexed: 152), which is byte 19, bit 0 (MSB)
        
        guard data.count >= 20 else { return false }
        
        // Verify version is 2
        let version = (data[0] & 0xFC) >> 2
        guard version == 2 else { return false }
        
        // Purpose 1 consent is at bit 152
        // Byte index: 152 / 8 = 19
        // Bit index within byte: 152 % 8 = 0 (MSB)
        let purposesByte = data[19]
        let purpose1Consent = (purposesByte & 0x80) != 0
        
        return purpose1Consent
    }
    
    /// Decode base64url string to Data
    private static func base64URLDecode(_ string: String) -> Data? {
        var base64 = string
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        
        // Add padding if needed
        let remainder = base64.count % 4
        if remainder > 0 {
            base64 += String(repeating: "=", count: 4 - remainder)
        }
        
        return Data(base64Encoded: base64)
    }
    
    /// Quick check if TCF string indicates consent for ads
    public static func hasAdConsent(_ tcfV2: String?, gdprApplies: Bool) -> Bool {
        let core = parse(tcfV2: tcfV2, gdprApplies: gdprApplies)
        
        // If GDPR doesn't apply, assume consent
        if !core.gdprApplies { return true }
        
        // Otherwise, require Purpose 1 consent
        return core.purpose1Consent
    }
}
