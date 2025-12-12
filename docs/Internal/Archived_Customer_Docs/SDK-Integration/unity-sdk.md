# Unity SDK Integration Guide (BYO-first, single-entry)

> Updated: 20 Nov 2025 -- Unity 2020.3+ (URP/HDRP compatible)  
> Scope: Bring-Your-Own demand mode with Config-as-Code, transparency proofs, and debugger overlay.

## TL;DR (single-entry quick start)
1. **Install package:** `Window > Package Manager > + > Add package from git URL...` -> `https://github.com/sbelakho2/Ad-Project.git?path=sdk/core/unity#main`
2. **Export a signed config:** `Apex Mediation > Config-as-Code` -> click **Export Mock Config** or paste your placements, set the signing key, and copy the JSON.
3. **Create a TextAsset:** in Unity, create `Assets/Config/apex_config.json`, paste the signed JSON, set `TextAsset` import settings to UTF-8.
4. **Drop `ApexMediationEntryPoint` into your bootstrap scene:** assign the TextAsset, signing key, optional adapter credentials, and (if desired) keep "Attach Debugger Overlay in Editor" enabled.
5. **Press Play:** the component initializes the SDK, verifies the signature, spawns the Mediation Debugger (toggle with backquote `\``), and logs sanitized ad events to the Console.

The rest of this guide breaks each step down in detail.

---

## BYO-first at a glance
- Core package does not include vendor Unity SDKs. You control which vendor packages you import.
- Single entry point (`ApexMediationEntryPoint`) initializes once per session and centralizes consent.
- Sandbox flags let you iterate adapters quickly without changing code (DEBUG/dev builds only).
- Adapter Dev Kit (iOS/tvOS) and Android parity allow validating adapters without bundling them in core.

### Adapter Dev Kit (Unity)
See also (Developer‑Facing):
- Unity Adapter Dev Kit deep‑dive: `docs/Developer-Facing/DevKits/unity.md`
- Dev Kit index (all platforms): `docs/Developer-Facing/AdapterDevKit.md`

---

## 1. Installing the Unity package

| Method | Steps |
| --- | --- |
| **Git (recommended)** | `Window > Package Manager > + > Add package from git URL...` -> `https://github.com/sbelakho2/Ad-Project.git?path=sdk/core/unity#main` |
| **Local folder** | Clone this repo, then in Package Manager choose **Add package from disk...** and select `sdk/core/unity/package.json`. |

The package ships two asmdefs:
- `ApexMediation` (Runtime) -- C# facade + platform bridges + Config-as-Code runtime.
- `ApexMediation.Editor` -- Config-as-Code window, migration helpers, app-ads inspector hooks.

No vendor SDKs are bundled; publishers continue to control adapter binaries in their project.

---

## 2. Config-as-Code workflow

1. Open **Apex Mediation > Config-as-Code**.
2. Provide a signing key (any shared secret; store it securely).
3. Use **Export Mock Config** for a sample or paste your existing JSON into the export area.
4. Save the signed JSON into a `TextAsset` (e.g., `Assets/Config/apex_config.json`).
5. The same window can validate imports before shipping -- paste a JSON blob, press **Validate & Load**, and verify adapter/placement counts.

All configs are HMAC signed and verified inside the runtime via `ConfigCodec`. If the signature fails, initialization is aborted and a clear Editor error is logged.

---

## 3. Single entry point: `ApexMediationEntryPoint`

Add the new component (`Add Component > Apex Mediation > Apex Mediation Entry Point`) to a bootstrap GameObject that lives for the lifetime of your session.

### Serialized fields
- **Signed Config Json** -- TextAsset containing the signed Config-as-Code document.
- **Signing Key** -- The HMAC key used to export the JSON. This is required to verify integrity. (For production you can inject via scriptable secret.)
- **Initialize On Awake** -- If unchecked, call `InitializeIfNeeded()` manually (e.g., after your own consent gate).
- **Attach Debugger Overlay In Editor** -- Automatically spawns a persistent HUD that surfaces telemetry traces and transparency proofs. Toggle with backquote.
- **Log Ad Events To Console** -- When enabled, every `OnAdEvent` is mirrored to the Unity Console with sanitized metadata.
- **Adapter Credentials** -- Optional array; each entry captures a network ID plus key/value pairs. The component wires these into the runtime `AdapterConfigProvider` so you do not have to script per-network injection for local testing.

If the SDK is already initialized (for example from end-to-end tests), calling `InitializeIfNeeded()` is a no-op, keeping the "single point of entry" invariant intact.

---

## 4. Loading and showing ads via the facade

Once the entry point initializes successfully, interact exclusively through the static `ApexMediation` class. Typical usage:

```csharp
using Apex.Mediation;
using UnityEngine;

public sealed class PlacementController : MonoBehaviour
{
    [SerializeField] private string interstitialPlacement = "level-complete";
    [SerializeField] private string rewardedPlacement = "extra-life";

    private void OnEnable()
    {
        ApexMediation.OnAdEvent += HandleAdEvent;
    }

    private void OnDisable()
    {
        ApexMediation.OnAdEvent -= HandleAdEvent;
    }

    public void PreloadInterstitial()
    {
        ApexMediation.LoadInterstitial(interstitialPlacement, (success, error) =>
        {
            if (!success)
            {
                Debug.LogWarning($"Interstitial failed to load: {error}");
            }
        });
    }

    public void ShowInterstitial()
    {
        ApexMediation.ShowInterstitial(interstitialPlacement);
    }

    public void ShowRewarded()
    {
        ApexMediation.LoadRewarded(rewardedPlacement);
        ApexMediation.ShowRewarded(rewardedPlacement, (success, error) =>
        {
            if (!success)
            {
                Debug.LogWarning($"Rewarded show failed: {error}");
            }
        });
    }

    private static void HandleAdEvent(AdEventArgs evt)
    {
        Debug.Log($"[{evt.Adapter}] {evt.PlacementId} -> {evt.Type} ({evt.Message})");
    }
}
```

API snapshot:
- `ApexMediation.Initialize(ApexConfig config, Action<bool> onComplete = null)` -- invoked automatically by the entry point.
- `SetConsent(ConsentOptions options)` -- push updated consent snapshots (GDPR, GPP, US Privacy, COPPA, ATT). Auto-read is enabled by default but explicit calls always win.
- `SetNetworkConfig(string network, IReadOnlyDictionary<string, object?> creds)` -- supply credentials at runtime (e.g., from your encrypted storage).
- `LoadInterstitial/ShowInterstitial`, `LoadRewarded/ShowRewarded`, `LoadBanner/AttachBanner/DestroyBanner` -- format-specific helpers.
- `GetTelemetryTraces()` and `GetTransparencyProofs()` -- backing collections for debugger overlays or custom diagnostics.

> **Paid events:** BYO integrations rely on the publisher's own network payouts, so Unity exposes only the sanitized `OnAdEvent` stream. Contact support if you need standardized paid event exports.

---

## 5. BYO adapters (native or C#)

There are two common patterns for adapters in Unity:

- Native adapters (recommended)
  - Bring native iOS/Android adapters and vendor SDKs into your Unity project as platform plugins.
  - The Unity entry point forwards placement requests to the native cores (iOS/tvOS/Android), which call your native adapters.
  - Pros: best performance, full feature parity with native cores; no duplicate logic.

- Unity-side adapters (advanced)
  - Define `INetworkAdapter` interfaces in C# and implement wrappers around vendor Unity SDKs.
  - The entry point maps requests to your C# implementations.
  - Pros: all-Unity code; Cons: more work to keep parity and feature coverage.

Release safety: The ApexMediation Unity package does not include vendor SDKs. All vendor packages remain your choice and responsibility.

---

## 5. Transparency & debugging

- **Debugger Overlay:** Toggle with backquote once attached. Shows the latest 50 traces plus the head of the transparency ledger (hash snippet). Safe for development builds only.
- **Telemetry snapshot:** `ApexMediation.GetTelemetryTraces()` returns immutable data you can serialize or forward to your own tooling.
- **Transparency proofs:** Each load/show is chained via `TransparencyLedger`. Share the proof hashes with your ops team for auditability.

---

## 6. Runtime credentials & consent best practices

- Never hardcode API keys in source control. Provide placeholder values in the entry point for local testing, but inject real secrets via your own secure store before shipping.
- When using `Adapter Credentials` on the entry point, note that they are serialized inside the scene/prefab. Use them only for mock networks or development values.
- If you maintain your own consent UI, call `ApexMediation.SetConsent` whenever the user updates preferences. The mediator forwards normalized strings/flags to every adapter and to any active S2S bridge.

---

## 7. Sandbox flags (adapter whitelist & force adapter pipeline)

In development, you can narrow testing to specific adapters and force the adapter pipeline (bypassing server-to-server in eligible modes) to keep iterations deterministic.

```csharp
using Apex.Mediation;

public class SandboxControls : MonoBehaviour
{
    void Start()
    {
        // Force client adapters even if S2S is eligible
        SdkBridge.SetSandboxForceAdapterPipeline(true);

        // Restrict to a single adapter, e.g. admob
        SdkBridge.SetSandboxAdapterWhitelist(new [] { "admob" });
    }
}
```

These flags affect only dev/test builds. Production builds should not call them.

---

## 8. Config-as-Code tips

- **Signing keys:** Keep the signing key outside of source control. For CI, set it via environment variables and feed it into the editor window using scripting, or supply it through the entry point at runtime.
- **Diff-friendly reviews:** Commit the exported JSON (with signature) next to your Unity scenes and treat it like any other asset. Because it is camelCase and sorted, diffs are stable.
- **Validation:** Run `sdk/core/unity/scripts/check_package_constraints.sh` locally or in CI to make sure the runtime stays under the 100KB compressed budget and all .NET tests pass before pushing.

---

## 9. Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `Config signature mismatch` | Signing key in entry point doesn't match export key | Re-export with the same key or update the entry point field |
| `SDK not initialized` when calling load/show | Entry point disabled or failed early | Check Console for errors, ensure `Initialize On Awake` is enabled or invoke `InitializeIfNeeded()` manually |
| Debugger overlay missing | Overlay disabled or stripped | Enable "Attach Debugger Overlay In Editor" or add `MediationDebuggerOverlay` manually in dev builds |
| CI fails `Unity Footprint Gate` | Runtime exceeded 100KB compressed or .NET tests failed | Run `./scripts/check_package_constraints.sh` locally, prune unused runtime code/assets, fix tests |

Need more help? Reach out via `support@apexmediation.ee` with your latest transparency proof hash and telemetry snapshot.

---

### Links
- Cross-platform Adapter Dev Kit guide: `docs/Developer-Facing/AdapterDevKit.md`
- Networked sandbox runbook (console/site/billing/VRA/soak): `docs/Internal/QA/networked-sandbox-runbook-2025-12-06.md`
