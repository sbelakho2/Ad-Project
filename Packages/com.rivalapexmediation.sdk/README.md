# Apex Mediation SDK for Unity

Production-ready Unity SDK for the Apex Mediation platform with server-to-server (S2S) auction flow, multiple ad formats, and comprehensive privacy compliance.

## Features

- **Ad Formats**: Interstitial, Rewarded, Banner, Rewarded Interstitial
- **S2S Auction**: Direct server communication with efficient bidding
- **Privacy Compliance**: GDPR, CCPA, COPPA support with ConsentManager
- **Platform Support**: iOS, Android, WebGL, Standalone
- **Unity Versions**: 2020.3 LTS through 2023.x
- **Zero Dependencies**: No external packages required
- **Size Budget**: ≤ 300KB uncompressed DLL

## Installation

### Via Unity Package Manager (UPM)

1. Open Unity Editor
2. Window → Package Manager
3. Click "+" → "Add package from git URL"
4. Enter: `https://github.com/rivalapex/unity-sdk.git`

### Manual Installation

1. Download the latest release from [GitHub Releases](https://github.com/rivalapex/unity-sdk/releases)
2. Extract to `Packages/com.rivalapexmediation.sdk/` in your Unity project

## Quick Start

```csharp
using RivalApex.Mediation;
using UnityEngine;

public class AdManager : MonoBehaviour
{
    void Start()
    {
        // Create SDK configuration
        var config = ScriptableObject.CreateInstance<SDKConfig>();
        config.AppId = "your-app-id";
        config.ApiKey = "your-api-key";
        config.TestMode = true;
        
        // Initialize SDK
        ApexMediation.Initialize(config, (success) =>
        {
            if (success)
            {
                Debug.Log("SDK initialized successfully");
                LoadInterstitial();
            }
            else
            {
                Debug.LogError("SDK initialization failed");
            }
        });
    }
    
    void LoadInterstitial()
    {
        ApexInterstitial.Load("your-placement-id", (error) =>
        {
            if (error == null)
            {
                Debug.Log("Interstitial loaded");
                ShowInterstitial();
            }
            else
            {
                Debug.LogError($"Failed to load: {error.Code}");
            }
        });
    }
    
    void ShowInterstitial()
    {
        ApexInterstitial.Show("your-placement-id",
            onShown: (error) =>
            {
                if (error != null)
                {
                    Debug.LogError($"Failed to show: {error.Code}");
                }
            },
            onClosed: () =>
            {
                Debug.Log("Interstitial closed");
            }
        );
    }
}
```

## Documentation

- [Integration Guide](Documentation~/IntegrationGuide.md)
- [API Reference](Documentation~/APIReference.md)
- [Consent Management](Documentation~/ConsentManagement.md)
- [Debugging](Documentation~/Debugging.md)

## Support

- Documentation: https://apexmediation.ee/docs
- Email: support@apexmediation.ee
- GitHub Issues: https://github.com/rivalapex/unity-sdk/issues

## License

© 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ. All rights reserved.
