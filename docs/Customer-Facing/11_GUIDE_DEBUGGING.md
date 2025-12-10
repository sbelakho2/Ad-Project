# Debugging Guide

## Mediation Debugger

The Mediation Debugger is a built-in tool in the SDK that helps you verify your integration without leaving your app.

### How to Open

**Android:**
```kotlin
// Shows the in-app debug panel
BelAds.showDebugPanel(activity)
```

**iOS:**
```swift
// iOS currently supports programmatic debug info only
let info = BelAds.getDebugInfo()
print("SDK Debug Info: \(info)")
```

**Unity:**
```csharp
// Validate specific adapters programmatically
ApexMediation.ValidateAdapter("admob", credentials, result => {
    Debug.Log($"Adapter Status: {result.IsValid}");
});
```

### Features

*   **Integration Status**: Checks if the SDK and adapters are installed correctly.
*   **Network Status**: Shows which ad networks are configured and their initialization status.
*   **Test Ads**: Allows you to force load ads from specific networks to verify they are working.

## Test Mode

During development, you should always use Test Mode to avoid flagging your account for invalid traffic.

### Enabling Test Mode

You can enable test mode programmatically or via the Console.

**Programmatically:**
```kotlin
BelAds.setTestMode(true)
```

**Console:**
Go to **App Settings > Test Devices** and add your device's Advertising ID (GAID or IDFA).

## Common Issues

### Ads Not Loading

1.  **Check Internet**: Ensure the device has an active connection.
2.  **Check IDs**: Verify App ID and Placement ID match the Console exactly.
3.  **Check Waterfall**: Ensure at least one network in your waterfall has fill.
4.  **Check Logs**: Enable verbose logging to see detailed error messages.

### Low Fill Rate

1.  **Add More Networks**: Increasing competition usually improves fill rate.
2.  **Check Pricing**: If your floor prices are too high, networks may not bid.
3.  **Check Geo**: Ensure you have networks that perform well in your target user's geography.

### Adapter Version Mismatch

The Debugger will highlight if you are using an outdated adapter version. Always try to keep adapters in sync with the core SDK version.
