import Foundation
import ApexAdapterDevKit
import RivalApexMediationSDK

struct Args {
    var adapter = ""
    var appId = "test-app"
    var inter = "test_interstitial"
    var reward = "test_rewarded"
    var timeout: TimeInterval = 10
    var gdprApplies: Bool? = nil
    var tcfString: String? = nil
    var usPrivacy: String? = nil
    var coppa: Bool? = nil
    var lat: Bool? = nil
    var junitPath: String? = nil
}

@discardableResult
func parseArgs() -> Args {
    var a = Args()
    var it = CommandLine.arguments.dropFirst().makeIterator()
    while let k = it.next() {
        switch k {
        case "--adapter": a.adapter = it.next() ?? a.adapter
        case "--appId": a.appId = it.next() ?? a.appId
        case "--inter": a.inter = it.next() ?? a.inter
        case "--reward": a.reward = it.next() ?? a.reward
        case "--timeout": if let v = it.next(), let d = TimeInterval(v) { a.timeout = d }
        case "--gdpr": if let v = it.next() { a.gdprApplies = (v == "1" || v.lowercased() == "true") }
        case "--tcf": a.tcfString = it.next()
        case "--usPrivacy": a.usPrivacy = it.next()
        case "--coppa": if let v = it.next() { a.coppa = (v == "1" || v.lowercased() == "true") }
        case "--lat": if let v = it.next() { a.lat = (v == "1" || v.lowercased() == "true") }
        case "--junit": a.junitPath = it.next()
        case "-h", "--help":
            print("""
            Usage: apex-adapter-runner --adapter <name> [--appId <id>] [--inter <placement>] [--reward <placement>] [--timeout <sec>]
                   [--gdpr 0|1] [--tcf <string>] [--usPrivacy <usp>] [--coppa 0|1] [--lat 0|1] [--junit <path>]
            """)
            exit(0)
        default:
            break
        }
    }
    return a
}

func writeJUnit(path: String, adapter: String, ok: Bool, message: String?) {
    let ts = Date().timeIntervalSince1970
    var xml = """
    <?xml version="1.0" encoding="UTF-8"?>
    <testsuite name="ApexAdapterConformance" tests="1" failures="\(ok ? 0 : 1)" time="\(ts)">
      <testcase classname="ApexAdapterConformance" name="\(adapter)">
    """
    if !ok {
        xml += "<failure message=\"\(message ?? "failure")\"/>"
    }
    xml += """
      </testcase>
    </testsuite>
    """
    do { try xml.write(to: URL(fileURLWithPath: path), atomically: true, encoding: .utf8) } catch {
        fputs("[warn] failed to write JUnit XML: \(error)\n", stderr)
    }
}

@main
enum Runner {
    static func main() async {
        let args = parseArgs()
        guard !args.adapter.isEmpty else {
            fputs("[err] --adapter is required\n", stderr)
            exit(2)
        }

        // Apply consent if provided
        if args.gdprApplies != nil || args.tcfString != nil || args.usPrivacy != nil || args.coppa != nil || args.lat != nil {
            let payload = ConsentData(
                gdprApplies: args.gdprApplies,
                gdprConsentString: args.tcfString,
                ccpaUSPrivacyString: args.usPrivacy,
                coppa: args.coppa,
                limitAdTracking: args.lat
            )
            MediationSDK.shared.setConsent(payload)
        }

        let ctx = AdapterTestContext(appId: args.appId, placementInterstitial: args.inter, placementRewarded: args.reward, timeout: args.timeout)
        let suite = ConformanceSuite()
        var ok = true
        var msg: String? = nil
        do {
            try await suite.runAll(adapterName: args.adapter, registerAdapter: { /* BYO external registration before launch */ }, context: ctx)
            print("[ok] Conformance suite passed for adapter=\(args.adapter)")
        } catch {
            ok = false
            msg = String(describing: error)
            fputs("[fail] Conformance failed: \(msg!)\n", stderr)
        }
        if let junit = args.junitPath { writeJUnit(path: junit, adapter: args.adapter, ok: ok, message: msg) }
        exit(ok ? 0 : 1)
    }
}
