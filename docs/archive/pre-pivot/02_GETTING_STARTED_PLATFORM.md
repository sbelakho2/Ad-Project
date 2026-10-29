# Getting Started

This guide will help you integrate your first ad placement in **under 60 minutes**.

## Prerequisites

1.  **ApexMediation Account**: [Sign up here](https://console.apexmediation.ee) (or your deployed console URL).
2.  **Ad Network Account**: You need at least one active account with a supported network (e.g., AdMob, AppLovin, Meta).
3.  **Development Environment**:
    *   **Android**: Android Studio, API Level 21+
    *   **iOS**: Xcode 14+, iOS 12+
    *   **Unity**: Unity 2021.3+

## Step 1: Console Setup (15 mins)

1.  **Create an App**:
    *   Log in to the Apex Console.
    *   Go to **Apps** > **New App**.
    *   Enter your app name and platform (Android/iOS).
    *   *Copy the `App ID`.*

2.  **Create a Placement**:
    *   Go to **Placements** > **New Placement**.
    *   Select format (e.g., `Interstitial`).
    *   Name it (e.g., `LevelComplete_Interstitial`).
    *   *Copy the `Placement ID`.*

3.  **Configure a Network**:
    *   Go to **Networks**.
    *   Select a network (e.g., **AdMob**).
    *   Enter your credentials (see [BYO Network Setup](09_BYO_NETWORK_SETUP.md)).
    *   Map the Apex Placement to the Network's Unit ID.

## Step 2: SDK Integration (30 mins)

Choose your platform:

*   [Android Integration Guide](03_SDK_ANDROID.md)
*   [iOS Integration Guide](04_SDK_IOS.md)
*   [Unity Integration Guide](05_SDK_UNITY.md)
*   [Web Integration Guide](06_SDK_WEB.md)

**Minimal Example (Unity):**

```csharp
void Start() {
    // Initialize
    ApexMediation.Initialize("YOUR_APP_ID");
    
    // Load Ad
    ApexMediation.LoadInterstitial("YOUR_PLACEMENT_ID");
}

void ShowAd() {
    if (ApexMediation.IsInterstitialReady("YOUR_PLACEMENT_ID")) {
        ApexMediation.ShowInterstitial("YOUR_PLACEMENT_ID");
    }
}
```

## Step 3: Verification (15 mins)

1.  **Enable Test Mode**: In your SDK initialization, enable test mode to ensure you get fills without affecting production stats.
2.  **Run the App**: Build and run on a device or simulator.
3.  **Mediation Debugger**:
    *   Trigger the debugger (see SDK docs).
    *   Verify that the SDK initialized successfully.
    *   Verify that the network adapter is "Ready".
4.  **Check Console**:
    *   Go to the **Debugger** tab in the Apex Console.
    *   You should see your request appear in real-time.

## Next Steps

*   [Add more networks](09_BYO_NETWORK_SETUP.md) to increase competition.
*   [Set up payment details](12_PRICING_BILLING.md).
*   [Review release notes](14_RELEASE_NOTES_UPGRADE.md) before going live.
