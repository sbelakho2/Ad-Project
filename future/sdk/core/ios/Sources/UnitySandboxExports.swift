import Foundation

#if (DEBUG) || APEX_SANDBOX_ADAPTERS
#if os(iOS) || os(tvOS)

// Unity iOS/tvOS native entry points for sandbox-only controls.
// These are no-ops for Release builds because this file is compiled
// only under DEBUG or when APEX_SANDBOX_ADAPTERS is defined.

@_cdecl("apex_set_sandbox_adapter_whitelist")
public func apex_set_sandbox_adapter_whitelist(_ csvNames: UnsafePointer<CChar>?) {
    let list: [String]?
    if let c = csvNames {
        let csv = String(cString: c)
        let names = csv.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        list = names
    } else {
        list = nil
    }
    Task {
        await MediationSDK.shared.setSandboxAdapterWhitelist(list)
    }
}

@_cdecl("apex_set_sandbox_force_adapter_pipeline")
public func apex_set_sandbox_force_adapter_pipeline(_ enabled: Bool) {
    Task {
        await MediationSDK.shared.setSandboxForceAdapterPipeline(enabled)
    }
}

#endif
#endif
