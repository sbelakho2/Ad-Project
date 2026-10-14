import Foundation

public struct ConsentData: Equatable {
    public var gdprApplies: Bool?
    public var tcfString: String?
    public var usPrivacy: String?
    public var coppa: Bool?
    public init(gdprApplies: Bool? = nil, tcfString: String? = nil, usPrivacy: String? = nil, coppa: Bool? = nil) {
        self.gdprApplies = gdprApplies
        self.tcfString = tcfString
        self.usPrivacy = usPrivacy
        self.coppa = coppa
    }
}

final class ConsentManager {
    private let key = "ctv_sdk_consent"
    private var cached: ConsentData
    init() {
        let d = UserDefaults.standard
        let gdpr = d.object(forKey: key+"_gdpr") as? Bool
        let tcf = d.string(forKey: key+"_tcf")
        let usp = d.string(forKey: key+"_usp")
        let coppa = d.object(forKey: key+"_coppa") as? Bool
        cached = ConsentData(gdprApplies: gdpr, tcfString: tcf, usPrivacy: usp, coppa: coppa)
    }
    func set(_ data: ConsentData) {
        cached = data
        let d = UserDefaults.standard
        d.set(data.gdprApplies, forKey: key+"_gdpr")
        d.set(data.tcfString, forKey: key+"_tcf")
        d.set(data.usPrivacy, forKey: key+"_usp")
        d.set(data.coppa, forKey: key+"_coppa")
    }
    func get() -> ConsentData { cached }
}
