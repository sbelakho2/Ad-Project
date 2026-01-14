import Foundation
#if canImport(UIKit)
import UIKit

extension UIApplication {
    static func ram_topViewController(base: UIViewController? = UIApplication.shared.connectedScenes
        .compactMap { ($0 as? UIWindowScene)?.keyWindow }
        .first?
        .rootViewController) -> UIViewController? {
        if let nav = base as? UINavigationController {
            return ram_topViewController(base: nav.visibleViewController)
        }
        if let tab = base as? UITabBarController, let selected = tab.selectedViewController {
            return ram_topViewController(base: selected)
        }
        if let presented = base?.presentedViewController {
            return ram_topViewController(base: presented)
        }
        return base
    }
}
#endif
