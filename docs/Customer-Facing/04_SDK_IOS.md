# iOS SDK Integration

## Quick Start

### 1. Add Dependencies

#### CocoaPods

Add the following to your `Podfile`:

```ruby
platform :ios, '12.0'

target 'MyApp' do
  use_frameworks!
  
  pod 'ApexMediation', '~> 1.0.0'
  # Add adapters for your BYO networks
  pod 'ApexMediation/AdMob'
  pod 'ApexMediation/AppLovin'
end
```

Run `pod install`.

#### Swift Package Manager

1.  File > Add Packages...
2.  Enter URL: `https://github.com/sbelakho2/apex-mediation-platform`
3.  Select version `1.0.0`

### 2. Initialize SDK

In your `AppDelegate.swift` or `App` struct.

```swift
import ApexMediation

@main
class AppDelegate: UIResponder, UIApplicationDelegate {

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        
        // Initialize with default config
        BelAds.initialize(appId: "YOUR_APP_ID") { result in
            switch result {
            case .success:
                print("Apex SDK Initialized")
            case .failure(let error):
                print("Initialization failed: \(error)")
            }
        }
        
        return true
    }
}
```

### 3. Load & Show Interstitial

```swift
import ApexMediation

class ViewController: UIViewController {

    override func viewDidLoad() {
        super.viewDidLoad()
        // Load
        BelInterstitial.load(placementId: "PLACEMENT_ID") { result in
             switch result {
             case .success:
                 print("Ad Loaded")
             case .failure(let error):
                 print("Ad Failed: \(error)")
             }
        }
    }

    @IBAction func showAd() {
        // Show
        if BelInterstitial.isReady(placementId: "PLACEMENT_ID") {
            BelInterstitial.show(from: self, placementId: "PLACEMENT_ID")
        }
    }
}
```

## App Tracking Transparency (iOS 14+)

The SDK provides a helper to request IDFA permission.

```swift
// Check status
let status = BelAds.trackingAuthorizationStatus()

// Request permission
BelAds.requestTrackingAuthorization { status in
    if status == .authorized {
        print("IDFA authorized")
    }
}
```

**Info.plist Requirement:**
Add `NSUserTrackingUsageDescription`:
```xml
<key>NSUserTrackingUsageDescription</key>
<string>This identifier will be used to deliver personalized ads to you.</string>
```

## Advanced Configuration

You can customize the initialization using `SDKConfig`.

```swift
let config = SDKConfig(
    appId: "YOUR_APP_ID",
    configEndpoint: "https://api.apexmediation.ee", // Enterprise only
    auctionEndpoint: "https://api.apexmediation.ee", // Enterprise only
    telemetryEnabled: true,
    logLevel: .verbose,
    testMode: true, // Enable for development
    enableOmSdk: true // Enable Open Measurement for viewability
)

MediationSDK.shared.initialize(appId: "YOUR_APP_ID", configuration: config)
```

## SKAdNetwork

Ensure your `Info.plist` includes the SKAdNetwork IDs for all your ad partners to track conversions.

```xml
<key>SKAdNetworkItems</key>
<array>
  <dict>
    <key>SKAdNetworkIdentifier</key>
    <string>cstr6suwn9.skadnetwork</string>
  </dict>
  <!-- Add other network IDs here -->
</array>
```

## Debugging

Open the **Mediation Debugger**:

```swift
import ApexMediation

// Present from current view controller
DebugPanel.present(from: self)
```
