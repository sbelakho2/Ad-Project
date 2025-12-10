# Unity SDK Integration

## Quick Start

### 1. Import SDK

1.  Download the `ApexMediation.unitypackage` from the [Releases Page](https://github.com/sbelakho2/apex-mediation-platform/releases).
2.  In Unity, go to **Assets > Import Package > Custom Package**.
3.  Select the downloaded file and click **Import**.

### 2. Resolve Dependencies

The SDK uses the External Dependency Manager for Unity (EDM4U) to handle Android and iOS dependencies.

1.  Go to **Assets > External Dependency Manager > Android Resolver > Force Resolve**.
2.  Ensure no errors are reported in the Console.

### 3. Initialize SDK

Create a script (e.g., `AdsManager.cs`) and attach it to a GameObject in your first scene.

```csharp
using Apex.Mediation;
using UnityEngine;

public class AdsManager : MonoBehaviour
{
    [SerializeField] private ApexConfig sdkConfig; // Assign in Inspector

    void Start()
    {
        if (sdkConfig == null)
        {
            Debug.LogError("SDK Config not assigned!");
            return;
        }

        ApexMediation.Initialize(sdkConfig, OnInitializationComplete);
    }

    private void OnInitializationComplete(bool success)
    {
        if (success)
        {
            Debug.Log("Apex SDK Initialized");
        }
        else
        {
            Debug.LogError("Apex SDK Initialization Failed");
        }
    }
}
```

### 4. Create Configuration

1.  Right-click in the Project window.
2.  Select **Create > Apex Mediation > SDK Config**.
3.  Name the file (e.g., `MyGameConfig`).
4.  Select the file and set your **App ID** and other settings in the Inspector.
5.  Assign this config file to your `AdsManager` script.

### 5. Load & Show Ad

```csharp
public void LoadInterstitial()
{
    ApexMediation.LoadInterstitial("PLACEMENT_ID");
}

public void ShowInterstitial()
{
    if (ApexMediation.IsInterstitialReady("PLACEMENT_ID"))
    {
        ApexMediation.ShowInterstitial("PLACEMENT_ID");
    }
}
```

## Key Concepts

*   **ScriptableObject Config**: Configuration is handled via a ScriptableObject asset, allowing you to easily swap configs for different environments (Dev vs Prod).
*   **Platform Specific IDs**: You may have different App IDs and Placement IDs for Android and iOS. The `SDKConfig` asset allows you to override IDs per platform.
*   **Main Thread**: All SDK calls must be made from the main Unity thread.

## Integration Checklist

1.  [ ] Imported Unity Package.
2.  [ ] Resolved Android/iOS dependencies via EDM4U.
3.  [ ] Created `SDKConfig` asset and assigned App ID.
4.  [ ] Verified ads on Android device.
5.  [ ] Verified ads on iOS device.

## Debugging

Enable debug logging in your `SDKConfig` asset or via code:

```csharp
// Runtime override
ApexMediation.SetLogLevel(LogLevel.Verbose);
```

Open Debugger:

```csharp
ApexMediation.OpenDebugger();
```
