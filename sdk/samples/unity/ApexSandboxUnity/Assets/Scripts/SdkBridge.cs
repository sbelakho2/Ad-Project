// Copyright (c) 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ.
// Unity-side bridge to native iOS/Android SDKs with Editor/Unknown platform simulation.
using System;
using System.Runtime.InteropServices;
using UnityEngine;

namespace ApexSandboxUnity
{
    public static class SdkBridge
    {
        public static string UnityBridgeVersion = "1.0.0";

        // Events (exactly-once per show guarded in SandboxController)
        public static event Action<string, AdFormat> OnAdLoaded;
        public static event Action<string, AdFormat, AdError, string> OnAdFailedToLoad;
        public static event Action<string, AdFormat> OnAdShown;
        public static event Action<string, AdFormat, AdError, string> OnAdFailedToShow;
        public static event Action<string, AdFormat> OnAdClosed;
        public static event Action<string, double> OnUserEarnedReward;

        // State (simple cache flags for simulation)
        private static bool _initialized;
        private static readonly System.Random rng = new System.Random();

        // Public API
        public static void Initialize(string appId, bool testMode)
        {
            if (_initialized) { Debug.Log("[SdkBridge] Initialize: already initialized"); return; }
#if UNITY_IOS && !UNITY_EDITOR
            try
            {
                apex_init(appId, testMode);
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[SdkBridge] apex_init failed: {e.Message}");
            }
            _initialized = true;
            Debug.Log($"[SdkBridge] iOS Initialize appId={appId}, testMode={testMode}");
#elif UNITY_ANDROID && !UNITY_EDITOR
            _initialized = true;
            // Try to call native Android bridge if present
            try
            {
                using (var unityPlayer = new AndroidJavaClass("com.unity3d.player.UnityPlayer"))
                using (var activity = unityPlayer.GetStatic<AndroidJavaObject>("currentActivity"))
                using (var bridge = new AndroidJavaClass("ee.apexmediation.unity.Bridge"))
                {
                    bridge.CallStatic("initialize", activity, appId, testMode);
                }
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[SdkBridge] Android bridge initialize not available: {e.Message}");
            }
            Debug.Log($"[SdkBridge] Android Initialize appId={appId}, testMode={testMode}");
#else
            _initialized = true;
            Debug.Log($"[SdkBridge] Sim Initialize appId={appId}, testMode={testMode}");
#endif
        }

        public static void SetConsent(ConsentPayload consent)
        {
            Debug.Log($"[SdkBridge] SetConsent gdpr={consent.gdprApplies} ccpaOptOut={consent.ccpaOptOut} coppa={consent.coppa} testMode={consent.testMode} tcf={consent.tcfString ?? "-"}");
            // Forward to native layers if available
            try
            {
                var json = JsonUtility.ToJson(consent);
#if UNITY_IOS && !UNITY_EDITOR
                apex_set_consent(json);
#elif UNITY_ANDROID && !UNITY_EDITOR
                using (var bridge = new AndroidJavaClass("ee.apexmediation.unity.Bridge"))
                {
                    bridge.CallStatic("setConsent", json);
                }
#endif
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[SdkBridge] Native consent forward failed: {e.Message}");
            }
        }

        // Sandbox controls (BYO/testing): adapter whitelist and force adapter pipeline
        public static void SetSandboxAdapterWhitelist(string[] names)
        {
            try
            {
#if UNITY_IOS && !UNITY_EDITOR
                var payload = names != null ? string.Join(",", names) : "";
                apex_set_sandbox_adapter_whitelist(payload);
#elif UNITY_ANDROID && !UNITY_EDITOR
                using (var bridge = new AndroidJavaClass("ee.apexmediation.unity.Bridge"))
                {
                    bridge.CallStatic("setSandboxAdapterWhitelist", (object)names);
                }
#else
                Debug.Log($"[SdkBridge] Sandbox whitelist set: [{string.Join(",", names ?? new string[0])}]");
#endif
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[SdkBridge] setSandboxAdapterWhitelist not available: {e.Message}");
            }
        }

        public static void SetSandboxForceAdapterPipeline(bool enabled)
        {
            try
            {
#if UNITY_IOS && !UNITY_EDITOR
                apex_set_sandbox_force_adapter_pipeline(enabled);
#elif UNITY_ANDROID && !UNITY_EDITOR
                using (var bridge = new AndroidJavaClass("ee.apexmediation.unity.Bridge"))
                {
                    bridge.CallStatic("setSandboxForceAdapterPipeline", enabled);
                }
#else
                Debug.Log($"[SdkBridge] Sandbox force adapter pipeline: {enabled}");
#endif
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[SdkBridge] setSandboxForceAdapterPipeline not available: {e.Message}");
            }
        }

        public static void LoadInterstitial(string placementId)
        {
            if (!_initialized) { Debug.LogWarning("[SdkBridge] LoadInterstitial before initialize"); return; }
            SimulateLoad(placementId, AdFormat.Interstitial);
        }

        public static void ShowInterstitial(string placementId)
        {
            if (!_initialized) { Debug.LogWarning("[SdkBridge] ShowInterstitial before initialize"); return; }
            SimulateShow(placementId, AdFormat.Interstitial);
        }

        public static void LoadRewarded(string placementId)
        {
            if (!_initialized) { Debug.LogWarning("[SdkBridge] LoadRewarded before initialize"); return; }
            SimulateLoad(placementId, AdFormat.Rewarded);
        }

        public static void ShowRewarded(string placementId)
        {
            if (!_initialized) { Debug.LogWarning("[SdkBridge] ShowRewarded before initialize"); return; }
            SimulateShow(placementId, AdFormat.Rewarded);
        }

        public static void ShowBanner(string placementId)
        {
            Debug.Log($"[SdkBridge] ShowBanner {placementId} (simulated)");
        }

        public static void HideBanner()
        {
            Debug.Log("[SdkBridge] HideBanner (simulated)");
        }

        // Simulation helpers
        private static void SimulateLoad(string placementId, AdFormat format)
        {
            // Simulate async load
            var chance = rng.NextDouble();
            if (string.IsNullOrEmpty(placementId) || placementId.StartsWith("invalid"))
            {
                OnAdFailedToLoad?.Invoke(placementId, format, AdError.InvalidPlacement, "invalid placement id");
                return;
            }
            if (chance < 0.1)
            {
                OnAdFailedToLoad?.Invoke(placementId, format, AdError.Timeout, "timeout");
                return;
            }
            if (chance < 0.2)
            {
                OnAdFailedToLoad?.Invoke(placementId, format, AdError.NoFill, "no fill");
                return;
            }
            OnAdLoaded?.Invoke(placementId, format);
        }

        private static void SimulateShow(string placementId, AdFormat format)
        {
            // Simulate show with reward for rewarded
            OnAdShown?.Invoke(placementId, format);
            if (format == AdFormat.Rewarded)
            {
                OnUserEarnedReward?.Invoke(placementId, 1);
            }
            OnAdClosed?.Invoke(placementId, format);
        }

        // Native iOS externs (reserved for future wiring)
#if UNITY_IOS && !UNITY_EDITOR
        [DllImport("__Internal")] private static extern void apex_init(string appId, bool testMode);
        [DllImport("__Internal")] private static extern void apex_set_consent(string json);
        [DllImport("__Internal")] private static extern void apex_set_sandbox_adapter_whitelist(string csvNames);
        [DllImport("__Internal")] private static extern void apex_set_sandbox_force_adapter_pipeline(bool enabled);
        [DllImport("__Internal")] private static extern void apex_load_interstitial(string placementId);
        [DllImport("__Internal")] private static extern void apex_show_interstitial(string placementId);
        [DllImport("__Internal")] private static extern void apex_load_rewarded(string placementId);
        [DllImport("__Internal")] private static extern void apex_show_rewarded(string placementId);
#endif
    }
}
