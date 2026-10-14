import Foundation

/// User consent data for privacy regulations
public struct ConsentData: Codable, Equatable, Sendable {
    /// Whether GDPR applies to this user
    public var gdprApplies: Bool?
    
    /// IAB TCF consent string (if GDPR applies)
    public var gdprConsentString: String?
    
    /// Raw IAB US Privacy string (CCPA/CPRA)
    public var usPrivacyString: String?
    
    /// CCPA "Do Not Sell" opt-out flag
    public var ccpaOptOut: Bool
    
    /// COPPA (Children's Online Privacy Protection Act) flag
    public var coppa: Bool
    
    public init(
        gdprApplies: Bool? = nil,
        gdprConsentString: String? = nil,
        usPrivacyString: String? = nil,
        ccpaOptOut: Bool = false,
        coppa: Bool = false
    ) {
        self.gdprApplies = gdprApplies
        self.gdprConsentString = gdprConsentString
        self.usPrivacyString = usPrivacyString
        self.ccpaOptOut = ccpaOptOut
        self.coppa = coppa
    }
}

/// Manages user consent for privacy compliance
public final class ConsentManager {
    public static let shared = ConsentManager()
    
    private let userDefaultsKey = "apex_consent_data"
    private let storage: UserDefaults
    private var currentConsent: ConsentData
    private let lock = NSLock()
    
    init(storage: UserDefaults = .standard) {
        self.storage = storage
        if let data = storage.data(forKey: userDefaultsKey),
           let decoded = try? JSONDecoder().decode(ConsentData.self, from: data) {
            currentConsent = decoded
        } else {
            currentConsent = ConsentData()
        }
    }
    
    /// Set user consent and persist
    public func setConsent(_ consent: ConsentData) {
        lock.lock()
        currentConsent = consent
        persistLocked(consent)
        lock.unlock()
    }
    
    /// Get current consent
    public func getConsent() -> ConsentData {
        lock.lock()
        let consent = currentConsent
        lock.unlock()
        return consent
    }
    
    /// Get redacted consent info for debug panel (no PII)
    public func getRedactedConsentInfo() -> [String: Any] {
        let consent = getConsent()
        return [
            "gdprApplies": consent.gdprApplies ?? false,
            "gdprConsentString": consent.gdprConsentString != nil ? "<redacted>" : "none",
            "usPrivacyString": consent.usPrivacyString != nil ? "<redacted>" : "none",
            "ccpaOptOut": consent.ccpaOptOut,
            "coppa": consent.coppa
        ]
    }
    
    /// Check if personalized ads are allowed based on consent
    public func canShowPersonalizedAds() -> Bool {
        // COPPA users cannot see personalized ads
        let consent = getConsent()
        if consent.coppa {
            return false
        }
        
        // CCPA opt-out means no personalized ads
        if ccpaOptedOut(consent) {
            return false
        }
        
        // GDPR: a TCF v2 string must carry Purpose 1 consent; a missing,
        // malformed, or truncated string refuses personalization.
        if let gdprApplies = consent.gdprApplies, gdprApplies {
            guard let raw = consent.gdprConsentString, !raw.isEmpty else {
                return false
            }
            return TCFParser.parse(tcfV2: raw, gdprApplies: true).purpose1Consent
        }
        
        // Default: allow personalized ads
        return true
    }
    
    /// Convert consent to metadata for ad requests
    public func toAdRequestMetadata() -> [String: Any] {
        var metadata: [String: Any] = [:]

        let consent = getConsent()
        let limitAdTracking = !canShowPersonalizedAds()

        if let gdprApplies = consent.gdprApplies {
            metadata["gdpr"] = gdprApplies ? 1 : 0
            metadata["gdpr_applies"] = gdprApplies ? 1 : 0
        }

        if let consentString = consent.gdprConsentString, !consentString.isEmpty {
            metadata["gdpr_consent"] = consentString
        }

        if let usp = normalizedUsPrivacyString(consent) {
            metadata["us_privacy"] = usp
        }

        if consent.coppa {
            metadata["coppa"] = 1
        }

        metadata["limit_ad_tracking"] = limitAdTracking ? 1 : 0

        return metadata
    }
    
    /// Convert consent to adapter-facing state payload
    public func toAdapterConsentPayload(attStatusProvider: () -> ATTStatus = { .notDetermined }) -> [String: Any] {
        let consent = getConsent()
        let usPrivacy = normalizedUsPrivacyString(consent)
        let limitAdTracking = !canShowPersonalizedAds()
            let consentState = ConsentState(
            gdprApplies: consent.gdprApplies,
            iabTCFv2: consent.gdprConsentString,
            iabUSPrivacy: usPrivacy,
            coppa: consent.coppa,
            attStatus: attStatusProvider(),
            limitAdTracking: limitAdTracking,
            privacySandboxOptIn: nil,
            advertisingId: nil,
            appSetId: nil
        )
        var payload: [String: Any] = [
            "coppa": consentState.coppa,
            "att_status": consentState.attStatus.rawValue,
            "limit_ad_tracking": consentState.limitAdTracking
        ]
        if let applies = consentState.gdprApplies {
            payload["gdpr_applies"] = applies ? 1 : 0
        }
        if let tcf = consentState.iabTCFv2, !tcf.isEmpty {
            payload["iab_tcf_v2"] = tcf
        }
        if let usp = consentState.iabUSPrivacy, !usp.isEmpty {
            payload["iab_us_privacy"] = usp
            payload["us_privacy"] = usp
        }
        return payload
    }
    
    private func persistLocked(_ consent: ConsentData) {
        if let encoded = try? JSONEncoder().encode(consent) {
            storage.set(encoded, forKey: userDefaultsKey)
        }
    }
    
    /// Clear all consent data (for testing)
    public func clear() {
        lock.lock()
        currentConsent = ConsentData()
        storage.removeObject(forKey: userDefaultsKey)
        lock.unlock()
    }

    private func normalizedUsPrivacyString(_ consent: ConsentData) -> String? {
        if let raw = consent.usPrivacyString?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty {
            return raw
        }
        return consent.ccpaOptOut ? "1YNN" : "1YYN"
    }

    private func ccpaOptedOut(_ consent: ConsentData) -> Bool {
        if consent.ccpaOptOut { return true }
        guard let raw = consent.usPrivacyString?.trimmingCharacters(in: .whitespacesAndNewlines), raw.count >= 2 else {
            return false
        }
        let flag = raw[raw.index(raw.startIndex, offsetBy: 1)]
        return flag == "Y" || flag == "y"
    }
}

    extension ConsentManager: @unchecked Sendable {}
