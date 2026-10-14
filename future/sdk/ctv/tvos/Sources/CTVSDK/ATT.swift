import Foundation
#if os(tvOS) && canImport(AppTrackingTransparency)
import AppTrackingTransparency
#endif

public enum TrackingAuthorizationStatus {
    case notDetermined
    case restricted
    case denied
    case authorized
    case unavailable
}

public enum ATTHelper {
    public static func status() -> TrackingAuthorizationStatus {
        #if os(tvOS) && canImport(AppTrackingTransparency)
        if #available(tvOS 14.0, *) {
            switch ATTrackingManager.trackingAuthorizationStatus {
            case .notDetermined: return .notDetermined
            case .restricted: return .restricted
            case .denied: return .denied
            case .authorized: return .authorized
            @unknown default: return .restricted
            }
        }
        #endif
        return .authorized
    }

    public static func requestAuthorization(completion: @escaping (TrackingAuthorizationStatus) -> Void) {
        #if os(tvOS) && canImport(AppTrackingTransparency)
        if #available(tvOS 14.0, *) {
            let current = ATTrackingManager.trackingAuthorizationStatus
            if current != .notDetermined {
                completion(status())
                return
            }
            ATTrackingManager.requestTrackingAuthorization { _ in
                completion(status())
            }
            return
        }
        #endif
        completion(.authorized)
    }
}
