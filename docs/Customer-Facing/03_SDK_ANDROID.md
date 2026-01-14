# Android SDK Integration

## Quick Start

### 1. Add Dependencies

Add the repository and dependency to your `build.gradle` (app level):

```groovy
repositories {
    mavenCentral()
    // TODO: SDK distribution URL to be confirmed before GA
    maven { url "https://sdk.apexmediation.ee/android" }
}

dependencies {
    implementation 'ee.apexmediation.sdk:core:1.0.0'
    // Add adapters for your BYO networks
    implementation 'ee.apexmediation.sdk:adapter-admob:1.0.0'
    implementation 'ee.apexmediation.sdk:adapter-applovin:1.0.0'
}
```

### 2. Initialize SDK

In your `Application` class or main `Activity`. You can configure the SDK using the `SDKConfig` builder.

```kotlin
// In your Application class or main Activity.
class MyApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        
        // Create configuration
        val config = SDKConfig.Builder()
            .appId("YOUR_APP_ID")
            .testMode(BuildConfig.DEBUG) // Enable test mode for debug builds
            .logLevel(LogLevel.INFO)
            .telemetryEnabled(true)
            .build()

        // Initialize
        BelAds.initialize(this, "YOUR_APP_ID", config)
    }
}
```

### 3. Load & Show Interstitial

Use `BelInterstitial` for interstitial ads.

```kotlin
// Load
BelInterstitial.load(context, "PLACEMENT_ID", object : AdLoadCallback {
    override fun onAdLoaded(placementId: String) {
        Log.d("Apex", "Ad loaded for $placementId")
    }

    override fun onAdFailedToLoad(placementId: String, error: Throwable) {
        Log.e("Apex", "Ad failed: ${error.message}")
    }
})

// Show (usually in response to a user action or game event)
if (BelInterstitial.show(activity)) {
    Log.d("Apex", "Ad shown")
} else {
    Log.d("Apex", "Ad not ready")
}
```

## Advanced Configuration

The `SDKConfig.Builder` offers several advanced options:

| Option | Description | Default |
| :--- | :--- | :--- |
| `testMode(Boolean)` | Enables test ads and logging. **Disable in production.** | `false` |
| `logLevel(LogLevel)` | Sets verbosity (`NONE`, `ERROR`, `INFO`, `VERBOSE`). | `INFO` |
| `telemetryEnabled(Boolean)` | Sends anonymous performance data to improve mediation. | `true` |
| `autoConsentReadEnabled(Boolean)` | Automatically reads TCF v2 strings from SharedPreferences. | `false` |
| `observabilityEnabled(Boolean)` | Enables distributed tracing for auction latency debugging. | `true` |

## Runtime Controls

You can modify some settings at runtime via `BelAds`:

```kotlin
// Enable verbose logging for debugging
BelAds.setLogLevel(LogLevel.VERBOSE)

// Toggle test mode (e.g., via a hidden debug menu)
BelAds.setTestMode(true)

// Set SDK API Key (required for RTB/auction endpoints)
// Generate keys in Console → Settings → SDK Keys
BelAds.setApiKey("sk_live_xxxx...")  // Use sk_test_ for development
```

## SDK API Keys

Your app requires an SDK API key to authenticate with Apex auction/RTB services.

1. Log in to the [Console](https://console.apexmediation.ee)
2. Navigate to **Settings → SDK Keys** (or **Apps → Your App → Keys**)
3. Create a key for your app, choosing:
   - **Test mode** (`sk_test_...`) for development/staging
   - **Live mode** (`sk_live_...`) for production
4. Copy the secret immediately (it's only shown once)
5. Set the key in your app before making ad requests

```kotlin
// In your Application.onCreate() or before first ad load
BelAds.setApiKey(BuildConfig.APEX_SDK_KEY)
```

Store keys securely:
- Use BuildConfig fields or encrypted preferences
- Never commit keys to source control
- Rotate keys periodically via Console
```

## ProGuard / R8

If you use code shrinking, add these rules to `proguard-rules.pro`:

```proguard
-keep class com.rivalapexmediation.sdk.** { *; }
-keep interface com.rivalapexmediation.sdk.** { *; }
-keep class com.iab.omid.** { *; }
```

## Debugging

Open the **Mediation Debugger** to verify your integration and adapter status:

```kotlin
// Launch the debugger activity
DebugPanel.open(context)
```
