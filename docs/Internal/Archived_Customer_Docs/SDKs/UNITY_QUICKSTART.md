# Unity SDK — Quickstart

_Last updated: 2025-11-20_

This guide walks you through integrating the Apex Mediation Unity SDK in under an hour. The package mirrors the Android/iOS runtime, operates without publisher secrets, and gives you identical consent, transparency, and debugging capabilities in BYO mode.

> **Production parity**: All public APIs shown below are the same ones exercised in `Samples~/MediationDemo`. Finish the sample scene before touching your game to keep your time-to-first-impression under 30 minutes.

---

## 1. Requirements

| Item | Value |
| --- | --- |
| Unity | 2020.3 LTS or newer (IL2CPP/Mono) |
| Platforms | iOS 12+, Android 5.0+, Desktop (editor play mode) |
| .NET profile | .NET Standard 2.1 |
| External SDKs | None bundled. Publishers import ad network SDKs directly per BYO policy. |

> **Privacy-first**: The SDK never stores network keys or secrets. You only supply account IDs, placement IDs, and consent state.

---

## 2. Install the package via UPM

1. In Unity, open **Window → Package Manager**.
2. Choose **Add package from disk…** and select `sdk/core/unity/package.json` from this repo.
3. (Optional) Import the sample scene via the Package Manager **Samples** tab → **Mediation Demo**.

You can also add the package by Git URL in `Packages/manifest.json`:

```jsonc
{
  "dependencies": {
    "com.apex.mediation": "file:../Ad-Project/sdk/core/unity"
  }
}
```

---

## 3. Create an `ApexConfig`

`ApexConfig` is a serializable object that defines your BYO placements, adapters, and operating mode. You can build it from a ScriptableObject, JSON, or inline C#.

```csharp
using Apex.Mediation.Core;

var config = new ApexConfig
{
    AppId = "publisher-app-id",
    Mode = SdkMode.BYO,
    EnableAutoConsentRead = false,
    EnableTelemetry = true,
    RenderTimeoutSeconds = 3.5d,
};

config.DefinePlacement("interstitial_home", PlacementFormat.Interstitial, floorCpm: 1.25);
config.DefinePlacement("rewarded_store", PlacementFormat.Rewarded, floorCpm: 3.00);

config.EnableAdapter(new NetworkAdapterDescriptor
{
    Name = "UnityAds",
    PackageId = "com.unity3d.ads",
    RequiredCredentialKeys = new[] { "gameId", "placementId" }
});
```

### Config-as-Code tooling

Use **Window → Apex Mediation → Config as Code** to:
- Import/export signed JSON via `ConfigCodec` (Ed25519 signatures enforced server-side).
- Inject per-network credentials at build time without storing them in the project.
- Validate placement schemas before shipping.

---

## 4. Drop the entry point in your first scene

Attach `ApexMediationEntryPoint` to a bootstrap GameObject (e.g., `AppStart`). The component wires initialization, consent auto-readers, and the in-editor debugger overlay.

```csharp
using Apex.Mediation;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using UnityEngine;

public class MediationBootstrap : MonoBehaviour
{
    [SerializeField] private ApexConfig config = default!; // assign via inspector or create at runtime

    private void Awake()
    {
        var consent = new ConsentOptions
        {
            GdprApplies = true,
            TcfString = PlayerPrefs.GetString("IABTCF_TCString", string.Empty),
            UsPrivacyString = PlayerPrefs.GetString("IABUSPrivacy_String", string.Empty),
            CoppaApplies = false,
        };

        ApexMediation.Initialize(config, success =>
        {
            Debug.Log($"Apex Mediation initialized: {success}");
        });

        ApexMediation.SetConsent(consent);
    }
}
```

> **Entry point tips**
> - Enable **Attach Debugger Overlay In Editor** on `ApexMediationEntryPoint` to see live traces and consent snapshots while iterating.
> - Set `Logger.SetDebug(true)` when working outside of the entry point (e.g., from unit tests).

---

## 5. Provide consent explicitly

BYO mode expects your CMP/application to remain the source of truth. Push updates whenever the user changes consent, ATT status, or COPPA flags.

```csharp
var consent = new ConsentOptions
{
    GdprApplies = gdprToggle.isOn,
    TcfString = cmp.LastTcfString,
    UsPrivacyString = cmp.LastUsPrivacy,
    CoppaApplies = parentalGateEnabled,
    AttStatus = attStatus, // "authorized", "denied", etc.
    LimitAdTracking = !analyticsOptIn,
};

ApexMediation.SetConsent(consent);
```

The SDK redacts this payload before handing it to adapters and surfaces it in the Debug Panel for transparency.

---

## 6. Load and show ads

### Interstitial

```csharp
ApexMediation.LoadInterstitial("interstitial_home", (success, error) =>
{
    if (!success)
    {
        Debug.LogWarning($"Interstitial failed: {error}");
        return;
    }

    ApexMediation.ShowInterstitial("interstitial_home", (shown, showError) =>
    {
        if (!shown)
        {
            Debug.LogWarning($"Show error: {showError}");
        }
    });
});
```

### Rewarded

```csharp
ApexMediation.LoadRewarded("rewarded_store", (success, error) =>
{
    if (success)
    {
        ApexMediation.ShowRewarded("rewarded_store", (shown, showError) =>
        {
            if (shown)
            {
                GrantReward();
            }
        });
    }
});
```

### Banners

```csharp
IntPtr nativeBannerSurface = bannerBridge.ResolveSurfaceHandle(bannerHost);
ApexMediation.LoadBanner("banner_footer", (success, error) =>
{
    if (success)
    {
        ApexMediation.AttachBanner("banner_footer", nativeBannerSurface);
    }
});

// Later, when hiding/destroying
ApexMediation.DestroyBanner("banner_footer");
```

`ResolveSurfaceHandle` represents the tiny bit of glue you write per platform (Android ViewGroup, iOS UIView). In the editor the mock bridge ignores the pointer, allowing you to exercise layout logic without native SDKs.

The SDK emits `ApexMediation.OnAdEvent` callbacks for every load/show/fail event so you can update UI, analytics, or custom telemetry.

---

## 7. Validate adapters in the editor

Open **Window → Apex Mediation → Adapter Credentials** to:
- Enter BYO account IDs/placement IDs for each network.
- Run the **Validate** action, which forwards credentials through the `AdapterValidator` to ensure required keys are present and, when supported, hits the native SDK validation endpoints.
- Export redacted configs for teammates without leaking secrets.

---

## 8. Debugging & transparency

- **Mediation Debugger Overlay**: Toggle the overlay in Play Mode to inspect placements, consent, adapters, and recent telemetry traces (`ApexMediationEntryPoint` spawns `MediationDebuggerOverlay`).
- **App-ads.txt Inspector**: From the editor window, run the inspector to confirm your publisher IDs appear on every domain.
- **Telemetry snapshots**: Call `ApexMediation.GetTelemetryTraces()` or `GetTransparencyProofs()` to display signed selection receipts in your own UI.

---

## 9. Platform-specific notes

| Platform | Notes |
| --- | --- |
| **iOS** | Enable ATT prompts yourself; pass the resulting status into `ConsentOptions.AttStatus`. Add `NSUserTrackingUsageDescription` and required network SKAdNetwork IDs. |
| **Android** | Forward GAID/Privacy Sandbox signals via your app if needed; the SDK will not query advertising IDs. Provide network SDK dependencies via Gradle (External Dependency Manager or manual). |
| **Standalone / Editor** | Uses the mock platform bridge; loads mock creatives and exercises telemetry without touching network SDKs. Ideal for CI smoke tests. |

---

## 10. Troubleshooting checklist

- **Initialization never completes** → Ensure an `ApexConfig` asset is assigned and that `AppId` is non-empty.
- **Ad fails with `Unknown adapter`** → Register the adapter via `config.EnableAdapter(...)` and verify the name matches `AdapterCatalog`.
- **Consent missing** → Call `ApexMediation.SetConsent` before the first load and after every CMP change.
- **No logs** → Call `Apex.Mediation.Internal.Logger.SetDebug(true)` in development builds.
- **Test mode** → Switch the SDK into BYO test mode via the Editor window or set `SdkMode.HYBRID` with mock placements while you finish accreditation.

Need more help? Reach us at support@apexmediation.ee with your transparency receipt hash and SDK version for faster triage.
