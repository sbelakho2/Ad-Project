# AdProject SDK API Reference

Complete API documentation for the AdProject Mediation SDK.

## Table of Contents

- [Getting Started](#getting-started)
- [Initialization](#initialization)
- [Banner Ads](#banner-ads)
- [Interstitial Ads](#interstitial-ads)
- [Rewarded Ads](#rewarded-ads)
- [Rewarded Interstitial Ads](#rewarded-interstitial-ads)
- [App Open Ads](#app-open-ads)
- [Signal Collection (S2S Bidding)](#signal-collection-s2s-bidding)
- [Consent Management](#consent-management)
- [Adapter Health Monitoring](#adapter-health-monitoring)
- [Error Handling](#error-handling)

---

## Getting Started

### Installation

#### Android (Gradle)

```gradle
dependencies {
    implementation 'io.adproject:sdk:1.0.0'
    
    // Optional: Network adapters
    implementation 'io.adproject:adapter-meta:1.0.0'
    implementation 'io.adproject:adapter-applovin:1.0.0'
    implementation 'io.adproject:adapter-unity:1.0.0'
}
```

#### iOS (CocoaPods)

```ruby
pod 'AdProjectSDK', '~> 1.0'

# Optional: Network adapters
pod 'AdProjectSDK/Meta', '~> 1.0'
pod 'AdProjectSDK/AppLovin', '~> 1.0'
pod 'AdProjectSDK/Unity', '~> 1.0'
```

#### React Native

```bash
npm install @adproject/react-native-sdk
# or
yarn add @adproject/react-native-sdk
```

#### Flutter

```yaml
dependencies:
  adproject_sdk: ^1.0.0
```

---

## Initialization

### Android (Kotlin)

```kotlin
import io.adproject.sdk.MediationSDK

class MyApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        
        MediationSDK.initialize(
            context = this,
            appId = "YOUR_APP_ID",
            testMode = BuildConfig.DEBUG,
            callback = object : InitializationCallback {
                override fun onInitialized(status: InitializationStatus) {
                    Log.d("AdProject", "SDK initialized: ${status.isReady}")
                    status.adapters.forEach { adapter ->
                        Log.d("AdProject", "${adapter.networkName}: ${adapter.status}")
                    }
                }
            }
        )
    }
}
```

### iOS (Swift)

```swift
import AdProjectSDK

@main
class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        
        MediationSDK.shared.initialize(
            appId: "YOUR_APP_ID",
            testMode: false
        ) { status in
            print("SDK initialized: \(status.isReady)")
            status.adapters.forEach { adapter in
                print("\(adapter.networkName): \(adapter.status)")
            }
        }
        
        return true
    }
}
```

### InitializationStatus

| Property | Type | Description |
|----------|------|-------------|
| `isReady` | `Boolean` | Whether SDK is ready for ad requests |
| `adapters` | `List<AdapterStatus>` | Status of each network adapter |

### AdapterStatus

| Property | Type | Description |
|----------|------|-------------|
| `networkName` | `String` | Network identifier (e.g., "meta", "applovin") |
| `status` | `InitStatus` | Initialization status enum |
| `sdkVersion` | `String?` | Network SDK version |
| `adapterVersion` | `String?` | Adapter version |
| `errorMessage` | `String?` | Error message if initialization failed |

### InitStatus Enum

| Value | Description |
|-------|-------------|
| `NOT_INITIALIZED` | Adapter not yet initialized |
| `INITIALIZING` | Initialization in progress |
| `INITIALIZED_SUCCESS` | Successfully initialized |
| `INITIALIZED_FAILURE` | Initialization failed |
| `DOES_NOT_APPLY` | Adapter doesn't require initialization |

---

## Banner Ads

### Loading a Banner

#### Android

```kotlin
val bannerAd = BannerAd(
    context = this,
    adUnitId = "YOUR_BANNER_AD_UNIT_ID",
    adSize = AdSize.BANNER
)

bannerAd.setAdListener(object : BannerAdListener {
    override fun onAdLoaded(result: AdLoadResult) {
        Log.d("AdProject", "Banner loaded from ${result.networkName}")
        // Add banner to your layout
        bannerContainer.addView(bannerAd.view)
    }
    
    override fun onAdFailedToLoad(error: AdError) {
        Log.e("AdProject", "Banner failed: ${error.message}")
    }
    
    override fun onAdClicked() {
        Log.d("AdProject", "Banner clicked")
    }
    
    override fun onAdImpression(data: ImpressionData) {
        Log.d("AdProject", "Impression: \$${data.revenue}")
    }
})

bannerAd.loadAd()
```

#### iOS

```swift
let bannerAd = BannerAd(
    adUnitId: "YOUR_BANNER_AD_UNIT_ID",
    adSize: .banner,
    rootViewController: self
)

bannerAd.delegate = self
bannerAd.load()

// BannerAdDelegate
extension ViewController: BannerAdDelegate {
    func bannerAd(_ bannerAd: BannerAd, didLoadWith result: AdLoadResult) {
        print("Banner loaded from \(result.networkName ?? "unknown")")
        view.addSubview(bannerAd.bannerView)
    }
    
    func bannerAd(_ bannerAd: BannerAd, didFailToLoadWith error: AdError) {
        print("Banner failed: \(error.message)")
    }
}
```

### Ad Sizes

| Size | Dimensions | Use Case |
|------|------------|----------|
| `BANNER` | 320×50 | Standard phone banner |
| `LARGE_BANNER` | 320×100 | Large phone banner |
| `MEDIUM_RECTANGLE` | 300×250 | Inline content |
| `FULL_BANNER` | 468×60 | Tablet banner |
| `LEADERBOARD` | 728×90 | Tablet leaderboard |

### Adaptive Banners

```kotlin
// Android
val adaptiveSize = AdaptiveBanner.getCurrentOrientationAdaptiveAdSize(context)
val bannerAd = BannerAd(context, adUnitId, adaptiveSize)

// iOS
let adaptiveSize = AdaptiveBanner.getCurrentOrientationAdaptiveAdSize()
let bannerAd = BannerAd(adUnitId: adUnitId, adSize: adaptiveSize, rootViewController: self)
```

---

## Interstitial Ads

### Loading

#### Android

```kotlin
private lateinit var interstitialAd: InterstitialAd

fun loadInterstitial() {
    interstitialAd = InterstitialAd(this, "YOUR_INTERSTITIAL_AD_UNIT_ID")
    
    interstitialAd.setAdListener(object : InterstitialAdListener {
        override fun onAdLoaded(result: AdLoadResult) {
            Log.d("AdProject", "Interstitial ready")
        }
        
        override fun onAdFailedToLoad(error: AdError) {
            Log.e("AdProject", "Load failed: ${error.message}")
        }
        
        override fun onAdOpened() {
            Log.d("AdProject", "Interstitial shown")
        }
        
        override fun onAdClosed() {
            Log.d("AdProject", "Interstitial closed")
            // Load next interstitial
            loadInterstitial()
        }
    })
    
    interstitialAd.load()
}
```

### Showing

```kotlin
fun showInterstitial() {
    if (interstitialAd.isReady) {
        interstitialAd.show(this)
    } else {
        Log.d("AdProject", "Interstitial not ready")
    }
}
```

---

## Rewarded Ads

### Loading

#### Android

```kotlin
private lateinit var rewardedAd: RewardedAd

fun loadRewarded() {
    rewardedAd = RewardedAd(this, "YOUR_REWARDED_AD_UNIT_ID")
    
    rewardedAd.setAdListener(object : RewardedAdListener {
        override fun onAdLoaded(result: AdLoadResult) {
            Log.d("AdProject", "Rewarded ready")
        }
        
        override fun onAdFailedToLoad(error: AdError) {
            Log.e("AdProject", "Load failed: ${error.message}")
        }
        
        override fun onRewardEarned(reward: Reward) {
            Log.d("AdProject", "Reward: ${reward.amount} ${reward.type}")
            grantReward(reward)
        }
        
        override fun onAdClosed() {
            loadRewarded() // Preload next
        }
    })
    
    rewardedAd.load()
}
```

#### iOS

```swift
var rewardedAd: RewardedAd?

func loadRewarded() {
    rewardedAd = RewardedAd(adUnitId: "YOUR_REWARDED_AD_UNIT_ID")
    rewardedAd?.delegate = self
    rewardedAd?.load()
}

// RewardedAdDelegate
func rewardedAd(_ rewardedAd: RewardedAd, didReward reward: Reward) {
    print("User earned \(reward.amount) \(reward.type)")
    grantReward(reward)
}
```

### Reward Object

| Property | Type | Description |
|----------|------|-------------|
| `type` | `String` | Reward type (e.g., "coins", "lives") |
| `amount` | `Int` | Reward quantity |

---

## Signal Collection (S2S Bidding)

### Collecting Signals

```kotlin
// Android
SignalCollector.collectSignals(context, object : SignalCollectionListener {
    override fun onSignalsCollected(signals: List<BidSignal>) {
        // Send signals to your server for auction
        signals.forEach { signal ->
            Log.d("AdProject", "${signal.networkName}: ${signal.token}")
        }
        
        sendToServer(signals)
    }
    
    override fun onSignalCollectionFailed(error: AdError) {
        Log.e("AdProject", "Signal collection failed: ${error.message}")
    }
})

// Collect from specific networks
SignalCollector.collectSignals(context, listOf("meta", "applovin"), listener)
```

```swift
// iOS
Task {
    let signals = try await SignalCollector.shared.collectSignals()
    signals.forEach { signal in
        print("\(signal.networkName): \(signal.token)")
    }
    sendToServer(signals)
}
```

### BidSignal Object

| Property | Type | Description |
|----------|------|-------------|
| `networkName` | `String` | Network identifier |
| `token` | `String` | Encrypted bid token |
| `timestamp` | `Long` | Collection timestamp (ms) |
| `expiresAt` | `Long` | Token expiration (ms) |

---

## Consent Management

### Setting Consent

```kotlin
// Android - GDPR
MediationSDK.setHasUserConsent(true)

// CCPA
MediationSDK.setDoNotSell(false)

// COPPA
MediationSDK.setIsAgeRestrictedUser(false)

// Full consent status
MediationSDK.setConsentStatus(
    ConsentStatus(
        hasConsent = true,
        consentString = "TCF_CONSENT_STRING",
        vendorConsents = mapOf(
            "meta" to true,
            "applovin" to true,
            "unity" to false
        )
    )
)
```

```swift
// iOS
MediationSDK.shared.setHasUserConsent(true)
MediationSDK.shared.setDoNotSell(false)
MediationSDK.shared.setIsAgeRestrictedUser(false)
```

---

## Adapter Health Monitoring

### Checking Adapter Status

```kotlin
// Android
val healthManager = AdapterHealthManager.getInstance()

// Get specific adapter
val metaHealth = healthManager.getAdapterHealth("meta")
Log.d("AdProject", "Meta status: ${metaHealth?.status}")
Log.d("AdProject", "Meta fill rate: ${metaHealth?.metrics?.fillRate}%")

// Get all adapters
val allHealth = healthManager.getAllAdapterHealth()
allHealth.forEach { (network, health) ->
    Log.d("AdProject", "$network: ${health.status} - Ready: ${health.isReady()}")
}

// Get ready adapters
val ready = healthManager.getReadyAdapters()
Log.d("AdProject", "Ready adapters: $ready")

// Listen for status changes
healthManager.addStatusListener(object : AdapterStatusListener {
    override fun onAdapterStatusChanged(networkName: String, health: AdapterHealth) {
        Log.d("AdProject", "$networkName changed to ${health.status}")
    }
})
```

### AdapterHealth Object

| Property | Type | Description |
|----------|------|-------------|
| `networkName` | `String` | Network identifier |
| `status` | `InitStatus` | Current status |
| `sdkVersion` | `String?` | Network SDK version |
| `adapterVersion` | `String?` | Adapter version |
| `capabilities` | `AdapterCapabilities` | Supported ad types |
| `metrics` | `AdapterMetrics` | Runtime performance metrics |

### AdapterMetrics Object

| Property | Type | Description |
|----------|------|-------------|
| `totalRequests` | `Int` | Total ad requests |
| `successfulFills` | `Int` | Successful fills |
| `noFills` | `Int` | No-fill responses |
| `errors` | `Int` | Error count |
| `timeouts` | `Int` | Timeout count |
| `avgLoadTimeMs` | `Double` | Average load time |
| `fillRate` | `Double` | Fill rate percentage |

---

## Error Handling

### AdError Object

| Property | Type | Description |
|----------|------|-------------|
| `code` | `String` | Error code |
| `message` | `String` | Human-readable message |
| `networkName` | `String?` | Network that caused error |

### Common Error Codes

| Code | Description |
|------|-------------|
| `NO_FILL` | No ads available |
| `NETWORK_ERROR` | Network connectivity issue |
| `TIMEOUT` | Request timed out |
| `INVALID_REQUEST` | Invalid ad request |
| `NOT_INITIALIZED` | SDK not initialized |
| `AD_EXPIRED` | Ad has expired |
| `ALREADY_SHOWING` | Ad already being displayed |

---

## Best Practices

1. **Initialize early** - Call `initialize()` in `Application.onCreate()` or `AppDelegate`
2. **Preload ads** - Load ads before you need them
3. **Handle errors gracefully** - Always implement error callbacks
4. **Respect user consent** - Set consent before loading ads
5. **Monitor adapter health** - Use health APIs for debugging
6. **Test in test mode** - Enable test mode during development

---

## Support

- Documentation: https://docs.adproject.io
- GitHub Issues: https://github.com/adproject/sdk/issues
- Email: support@adproject.io
