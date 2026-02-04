import UIKit

// MARK: - Adaptive Banner Support

/// Provides adaptive banner sizing that adjusts to device screen width,
/// matching Google AdMob's GADAdSize behavior.
public final class AdaptiveBanner {
    
    // MARK: - Standard Ad Sizes (Google AdMob compatible)
    
    public struct AdSize: Equatable {
        public let width: Int
        public let height: Int
        public let isAdaptive: Bool
        
        public init(width: Int, height: Int, isAdaptive: Bool = false) {
            self.width = width
            self.height = height
            self.isAdaptive = isAdaptive
        }
        
        /// Standard banner (320x50)
        public static let banner = AdSize(width: 320, height: 50)
        
        /// Large banner (320x100)
        public static let largeBanner = AdSize(width: 320, height: 100)
        
        /// Medium rectangle (300x250)
        public static let mediumRectangle = AdSize(width: 300, height: 250)
        
        /// Full banner (468x60)
        public static let fullBanner = AdSize(width: 468, height: 60)
        
        /// Leaderboard (728x90)
        public static let leaderboard = AdSize(width: 728, height: 90)
        
        /// Fluid size - adapts to content
        public static let fluid = AdSize(width: -1, height: -2)
        
        /// Invalid size
        public static let invalid = AdSize(width: 0, height: 0)
        
        public var cgSize: CGSize {
            CGSize(width: CGFloat(width), height: CGFloat(height))
        }
        
        public func isValid() -> Bool {
            return width > 0 && height > 0
        }
    }
    
    // MARK: - Adaptive Size Calculation
    
    /// Get adaptive banner size for current device orientation
    /// - Parameter width: Width to constrain the banner to, defaults to screen width
    /// - Returns: AdSize optimized for current orientation
    public static func getCurrentOrientationAdaptiveAdSize(width: CGFloat? = nil) -> AdSize {
        let screenWidth = width ?? UIScreen.main.bounds.width
        let orientation = UIDevice.current.orientation
        
        if orientation.isLandscape {
            return calculateAdaptiveSize(width: screenWidth, isLandscape: true)
        } else {
            return calculateAdaptiveSize(width: screenWidth, isLandscape: false)
        }
    }
    
    /// Get adaptive banner size for portrait orientation
    /// - Parameter width: Width to constrain the banner to, defaults to screen width
    /// - Returns: AdSize optimized for portrait
    public static func getPortraitAdaptiveAdSize(width: CGFloat? = nil) -> AdSize {
        let screenWidth = width ?? UIScreen.main.bounds.width
        return calculateAdaptiveSize(width: screenWidth, isLandscape: false)
    }
    
    /// Get adaptive banner size for landscape orientation
    /// - Parameter width: Width to constrain the banner to, defaults to screen width
    /// - Returns: AdSize optimized for landscape
    public static func getLandscapeAdaptiveAdSize(width: CGFloat? = nil) -> AdSize {
        let screenWidth = width ?? UIScreen.main.bounds.width
        return calculateAdaptiveSize(width: screenWidth, isLandscape: true)
    }
    
    /// Get inline adaptive banner size (variable height based on content)
    /// - Parameters:
    ///   - width: Width for the banner
    ///   - maxHeight: Maximum height constraint
    /// - Returns: AdSize with calculated dimensions
    public static func getInlineAdaptiveAdSize(width: CGFloat, maxHeight: CGFloat) -> AdSize {
        let calculatedHeight = min(calculateHeight(for: width), maxHeight)
        return AdSize(width: Int(width), height: Int(calculatedHeight), isAdaptive: true)
    }
    
    /// Get anchored adaptive banner size
    /// - Parameter width: Width for the banner
    /// - Returns: AdSize anchored to screen edge
    public static func getAnchoredAdaptiveAdSize(width: CGFloat) -> AdSize {
        let height = calculateHeight(for: width)
        return AdSize(width: Int(width), height: Int(height), isAdaptive: true)
    }
    
    // MARK: - Private Implementation
    
    /// Calculate adaptive banner height based on width
    /// Following Google AdMob's algorithm:
    /// - Full screen width: Height ranges from 50-90 dp
    /// - Based on device category and width
    private static func calculateAdaptiveSize(width: CGFloat, isLandscape: Bool) -> AdSize {
        let height = calculateHeight(for: width)
        
        // Landscape banners are typically shorter
        let adjustedHeight = isLandscape ? min(height, 50) : height
        
        return AdSize(width: Int(width), height: Int(adjustedHeight), isAdaptive: true)
    }
    
    /// Calculate banner height based on width (Google's algorithm approximation)
    private static func calculateHeight(for width: CGFloat) -> CGFloat {
        // Google's adaptive banner height calculation:
        // - Width < 400: 50dp
        // - Width 400-720: Scales from 50dp to 90dp
        // - Width > 720: 90dp (capped)
        
        if width < 400 {
            return 50
        } else if width >= 400 && width < 720 {
            // Linear interpolation from 50 to 90 as width goes from 400 to 720
            let ratio = (width - 400) / (720 - 400)
            return 50 + (40 * ratio)
        } else {
            return 90
        }
    }
    
    // MARK: - Device Detection
    
    /// Check if current device is a tablet
    public static var isTablet: Bool {
        UIDevice.current.userInterfaceIdiom == .pad
    }
    
    /// Check if current device is a phone
    public static var isPhone: Bool {
        UIDevice.current.userInterfaceIdiom == .phone
    }
    
    /// Get safe area insets for banner placement
    public static var safeAreaInsets: UIEdgeInsets {
        if #available(iOS 13.0, *) {
            if let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
               let window = scene.windows.first {
                return window.safeAreaInsets
            }
        }
        return .zero
    }
    
    /// Calculate full width accounting for safe area
    public static var fullWidthWithSafeArea: CGFloat {
        let screenWidth = UIScreen.main.bounds.width
        let insets = safeAreaInsets
        return screenWidth - insets.left - insets.right
    }
}

// MARK: - AdSize Extensions

public extension AdaptiveBanner.AdSize {
    
    /// Create custom size
    static func custom(width: Int, height: Int) -> AdaptiveBanner.AdSize {
        return AdaptiveBanner.AdSize(width: width, height: height)
    }
    
    /// Get aspect ratio
    var aspectRatio: CGFloat {
        guard height > 0 else { return 0 }
        return CGFloat(width) / CGFloat(height)
    }
    
    /// Convert to dictionary for serialization
    func toDict() -> [String: Any] {
        return [
            "width": width,
            "height": height,
            "isAdaptive": isAdaptive
        ]
    }
}

// MARK: - UIView Extension for Banner Sizing

public extension UIView {
    
    /// Set up constraints for adaptive banner
    func configureForAdaptiveBanner(_ adSize: AdaptiveBanner.AdSize) {
        translatesAutoresizingMaskIntoConstraints = false
        
        if adSize.isAdaptive {
            // Adaptive: set width to full width, constrain height
            widthAnchor.constraint(equalToConstant: CGFloat(adSize.width)).isActive = true
            heightAnchor.constraint(lessThanOrEqualToConstant: CGFloat(adSize.height)).isActive = true
        } else {
            // Fixed: set exact dimensions
            widthAnchor.constraint(equalToConstant: CGFloat(adSize.width)).isActive = true
            heightAnchor.constraint(equalToConstant: CGFloat(adSize.height)).isActive = true
        }
    }
}
