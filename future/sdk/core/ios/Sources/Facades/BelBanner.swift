#if canImport(UIKit)
import Foundation
import UIKit

/// Public facade for Banner ads, mirroring Android's BelBanner API.
/// Provides load/show/hide/destroy methods for persistent banner ad display.
@MainActor
public enum BelBanner {
    private static var activeBanners: [String: BannerView] = [:]
    
    /// Banner size options matching Android SDK
    public enum BannerSize {
        case standard      // 320x50
        case mediumRectangle // 300x250
        case leaderboard   // 728x90
        case adaptive      // Adapts to screen width
        
        @MainActor var dimensions: CGSize {
            switch self {
            case .standard:
                return CGSize(width: 320, height: 50)
            case .mediumRectangle:
                return CGSize(width: 300, height: 250)
            case .leaderboard:
                return CGSize(width: 728, height: 90)
            case .adaptive:
                let width = UIScreen.main.bounds.width
                let height: CGFloat = width > 728 ? 90 : 50
                return CGSize(width: width, height: height)
            }
        }
    }
    
    /// Banner position options
    public enum BannerPosition {
        case top
        case topLeft
        case topRight
        case bottom
        case bottomLeft
        case bottomRight
        case center
    }
    
    /// Create and load a banner ad
    /// - Parameters:
    ///   - placementId: Placement identifier
    ///   - size: Banner size
    ///   - position: Screen position
    ///   - viewController: View controller to attach banner to
    ///   - completion: Result callback with loaded ad or error
    public static func create(
        placementId: String,
        size: BannerSize,
        position: BannerPosition,
        in viewController: UIViewController,
        completion: @escaping (Result<Void, Error>) -> Void
    ) {
        // Clean up existing banner for this placement
        if let existing = activeBanners[placementId] {
            existing.removeFromSuperview()
            activeBanners.removeValue(forKey: placementId)
        }
        
        Task { @MainActor in
            do {
                let ad = try await MediationSDK.shared.loadAd(placementId: placementId)
                guard let ad = ad else {
                    completion(.failure(SDKError.noFill))
                    return
                }
                
                // Create banner view
                let bannerView = BannerView(
                    frame: .zero,
                    size: size,
                    position: position,
                    ad: ad
                )
                
                // Add to view hierarchy (initially hidden)
                viewController.view.addSubview(bannerView)
                bannerView.isHidden = true
                activeBanners[placementId] = bannerView
                
                completion(.success(()))
            } catch {
                completion(.failure(error))
            }
        }
    }
    
    /// Show a previously loaded banner
    public static func show(placementId: String) {
        guard let banner = activeBanners[placementId] else { return }
        banner.isHidden = false
    }
    
    /// Hide a banner without destroying it
    public static func hide(placementId: String) {
        guard let banner = activeBanners[placementId] else { return }
        banner.isHidden = true
    }
    
    /// Destroy a banner and remove from view hierarchy
    public static func destroy(placementId: String) {
        guard let banner = activeBanners.removeValue(forKey: placementId) else { return }
        banner.removeFromSuperview()
    }
    
    /// Check if banner is loaded and ready
    public static func isReady(placementId: String) -> Bool {
        return activeBanners[placementId] != nil
    }
    // MARK: - Internal BannerView

    private class BannerView: UIView {
        let ad: Ad
        let size: BelBanner.BannerSize
        let position: BelBanner.BannerPosition
        
        init(frame: CGRect, size: BelBanner.BannerSize, position: BelBanner.BannerPosition, ad: Ad) {
            self.ad = ad
            self.size = size
            self.position = position
            super.init(frame: frame)
            setupView()
        }
        
        required init?(coder: NSCoder) {
            fatalError("init(coder:) not implemented")
        }
        
        private func setupView() {
            translatesAutoresizingMaskIntoConstraints = false
            
            let dimensions = size.dimensions
            
            // Size constraints
            NSLayoutConstraint.activate([
                widthAnchor.constraint(equalToConstant: dimensions.width),
                heightAnchor.constraint(equalToConstant: dimensions.height)
            ])
            
            // Position constraints
            guard let superview = superview else { return }
            
            switch position {
            case .top:
                NSLayoutConstraint.activate([
                    topAnchor.constraint(equalTo: superview.safeAreaLayoutGuide.topAnchor),
                    centerXAnchor.constraint(equalTo: superview.centerXAnchor)
                ])
            case .topLeft:
                NSLayoutConstraint.activate([
                    topAnchor.constraint(equalTo: superview.safeAreaLayoutGuide.topAnchor),
                    leadingAnchor.constraint(equalTo: superview.leadingAnchor)
                ])
            case .topRight:
                NSLayoutConstraint.activate([
                    topAnchor.constraint(equalTo: superview.safeAreaLayoutGuide.topAnchor),
                    trailingAnchor.constraint(equalTo: superview.trailingAnchor)
                ])
            case .bottom:
                NSLayoutConstraint.activate([
                    bottomAnchor.constraint(equalTo: superview.safeAreaLayoutGuide.bottomAnchor),
                    centerXAnchor.constraint(equalTo: superview.centerXAnchor)
                ])
            case .bottomLeft:
                NSLayoutConstraint.activate([
                    bottomAnchor.constraint(equalTo: superview.safeAreaLayoutGuide.bottomAnchor),
                    leadingAnchor.constraint(equalTo: superview.leadingAnchor)
                ])
            case .bottomRight:
                NSLayoutConstraint.activate([
                    bottomAnchor.constraint(equalTo: superview.safeAreaLayoutGuide.bottomAnchor),
                    trailingAnchor.constraint(equalTo: superview.trailingAnchor)
                ])
            case .center:
                NSLayoutConstraint.activate([
                    centerXAnchor.constraint(equalTo: superview.centerXAnchor),
                    centerYAnchor.constraint(equalTo: superview.centerYAnchor)
                ])
            }
            
            // Placeholder rendering (debug mode)
            #if DEBUG
            backgroundColor = .belBannerPlaceholder
            let label = UILabel()
            label.text = "Banner Ad\n\(ad.networkName)"
            label.textAlignment = .center
            label.numberOfLines = 0
            label.font = .systemFont(ofSize: 12)
            label.translatesAutoresizingMaskIntoConstraints = false
            addSubview(label)
            NSLayoutConstraint.activate([
                label.centerXAnchor.constraint(equalTo: centerXAnchor),
                label.centerYAnchor.constraint(equalTo: centerYAnchor),
                label.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 8),
                label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -8)
            ])
            #endif
        }
        
        override func didMoveToSuperview() {
            super.didMoveToSuperview()
            if superview != nil {
                setupView()
            }
        }
    }
}

#endif

#if canImport(UIKit)
private extension UIColor {
    /// Provides a placeholder tint that is valid on both iOS and tvOS.
    static var belBannerPlaceholder: UIColor {
        #if os(tvOS)
        return UIColor(red: 0.24, green: 0.24, blue: 0.28, alpha: 1.0)
        #else
        if #available(iOS 13.0, *) {
            return .systemGray5
        }
        return UIColor(white: 0.9, alpha: 1.0)
        #endif
    }
}
#endif
