// ApexSandboxUnity single-scene controller with Init/Load/Show (Interstitial/Rewarded), optional Banner, consent toggles, and status console.
using System;
using UnityEngine;

namespace ApexSandboxUnity
{
    public class SandboxController : MonoBehaviour
    {
        [Header("Config")]
        [SerializeField] private string appId = "sandbox-app-unity";
        [SerializeField] private string interstitialPlacement = "3d1094ab-a85b-4737-a749-d8a153a0f026";
        [SerializeField] private string rewardedPlacement = "074b2dc7-3173-4da0-aba0-250f3f129df1";
        [SerializeField] private string bannerPlacement = "f6e7aa9b-09c5-4644-bf56-f8ab781ac62d";

        [Header("Consent/Test")]
        [SerializeField] private bool testMode = true;
        [SerializeField] private bool gdprApplies = false;
        [SerializeField] private bool ccpaOptOut = false;
        [SerializeField] private bool coppa = false;
        [SerializeField] private string tcfString = null;

        [Header("State (read-only)")]
        [SerializeField] private bool initialized = false;
        [SerializeField] private bool interstitialLoaded = false;
        [SerializeField] private bool rewardedLoaded = false;
        [SerializeField] private bool presenting = false;
        [SerializeField] private bool bannerVisible = false;
        [SerializeField] private string lastError = "";

        private Logger _log = new Logger(200);
        private Vector2 _scroll;

        void Awake()
        {
            // Attempt to load config from Resources/SandboxConfig.json if present
            var text = Resources.Load<TextAsset>("SandboxConfig");
            if (text != null)
            {
                try
                {
                    var cfg = JsonUtility.FromJson<Config>(text.text);
                    if (!string.IsNullOrEmpty(cfg.appId)) appId = cfg.appId;
                    if (!string.IsNullOrEmpty(cfg.placements.interstitialA)) interstitialPlacement = cfg.placements.interstitialA;
                    if (!string.IsNullOrEmpty(cfg.placements.rewardedA)) rewardedPlacement = cfg.placements.rewardedA;
                    if (!string.IsNullOrEmpty(cfg.placements.bannerA)) bannerPlacement = cfg.placements.bannerA;
                    if (cfg.consent != null)
                    {
                        gdprApplies = cfg.consent.gdpr;
                        ccpaOptOut = cfg.consent.ccpa;
                        coppa = cfg.consent.coppa;
                        testMode = cfg.testMode;
                    }
                }
                catch (Exception e)
                {
                    _log.Log($"Config parse failed: {e.Message}");
                }
            }
        }

        void OnEnable()
        {
            SdkBridge.OnAdLoaded += HandleAdLoaded;
            SdkBridge.OnAdFailedToLoad += HandleAdFailedToLoad;
            SdkBridge.OnAdShown += HandleAdShown;
            SdkBridge.OnAdFailedToShow += HandleAdFailedToShow;
            SdkBridge.OnAdClosed += HandleAdClosed;
            SdkBridge.OnUserEarnedReward += HandleUserEarnedReward;
        }

        void OnDisable()
        {
            SdkBridge.OnAdLoaded -= HandleAdLoaded;
            SdkBridge.OnAdFailedToLoad -= HandleAdFailedToLoad;
            SdkBridge.OnAdShown -= HandleAdShown;
            SdkBridge.OnAdFailedToShow -= HandleAdFailedToShow;
            SdkBridge.OnAdClosed -= HandleAdClosed;
            SdkBridge.OnUserEarnedReward -= HandleUserEarnedReward;
        }

        // GUI
        void OnGUI()
        {
            var w = Mathf.Min(Screen.width - 20, 680);
            GUILayout.BeginArea(new Rect(10, 10, w, Screen.height - 20), GUI.skin.box);
            GUILayout.Label($"Apex Sandbox Unity — Bridge v{SdkBridge.UnityBridgeVersion}");
            GUILayout.Space(4);

            GUILayout.Label(initialized ? "Status: Initialized" : "Status: Not Initialized");
            GUILayout.BeginHorizontal();
            if (GUILayout.Button(initialized ? "Re-Init (idempotent)" : "Initialize", GUILayout.Height(28)))
            {
                Initialize();
            }
            if (GUILayout.Button("Apply Consent", GUILayout.Height(28)))
            {
                ApplyConsent();
            }
            GUILayout.EndHorizontal();

            // Toggles
            GUILayout.BeginHorizontal();
            testMode = GUILayout.Toggle(testMode, "Test Mode");
            gdprApplies = GUILayout.Toggle(gdprApplies, "GDPR applies");
            ccpaOptOut = GUILayout.Toggle(ccpaOptOut, "CCPA opt-out");
            coppa = GUILayout.Toggle(coppa, "COPPA");
            GUILayout.EndHorizontal();

            GUILayout.Space(6);
            GUILayout.Label("Interstitial");
            GUILayout.BeginHorizontal();
            GUI.enabled = initialized;
            if (GUILayout.Button("Load", GUILayout.Height(24)))
            {
                interstitialLoaded = false;
                SdkBridge.LoadInterstitial(interstitialPlacement);
                _log.Log($"Load Interstitial → {interstitialPlacement}");
            }
            GUI.enabled = initialized && interstitialLoaded && !presenting;
            if (GUILayout.Button("Show", GUILayout.Height(24)))
            {
                presenting = true;
                SdkBridge.ShowInterstitial(interstitialPlacement);
            }
            GUI.enabled = true;
            GUILayout.FlexibleSpace();
            GUILayout.Label(interstitialLoaded ? "Loaded" : "Not loaded");
            GUILayout.EndHorizontal();

            GUILayout.Label("Rewarded");
            GUILayout.BeginHorizontal();
            GUI.enabled = initialized;
            if (GUILayout.Button("Load", GUILayout.Height(24)))
            {
                rewardedLoaded = false;
                SdkBridge.LoadRewarded(rewardedPlacement);
                _log.Log($"Load Rewarded → {rewardedPlacement}");
            }
            GUI.enabled = initialized && rewardedLoaded && !presenting;
            if (GUILayout.Button("Show", GUILayout.Height(24)))
            {
                presenting = true;
                SdkBridge.ShowRewarded(rewardedPlacement);
            }
            GUI.enabled = true;
            GUILayout.FlexibleSpace();
            GUILayout.Label(rewardedLoaded ? "Loaded" : "Not loaded");
            GUILayout.EndHorizontal();

            GUILayout.Space(4);
            GUILayout.BeginHorizontal();
            var newBanner = GUILayout.Toggle(bannerVisible, "Banner Visible");
            if (newBanner != bannerVisible)
            {
                bannerVisible = newBanner;
                if (bannerVisible) SdkBridge.ShowBanner(bannerPlacement); else SdkBridge.HideBanner();
            }
            GUILayout.FlexibleSpace();
            GUILayout.EndHorizontal();

            if (!string.IsNullOrEmpty(lastError))
            {
                var color = GUI.color;
                GUI.color = Color.red;
                GUILayout.Label($"Last error: {lastError}");
                GUI.color = color;
            }

            GUILayout.Space(6);
            GUILayout.Label("Console:");
            _scroll = GUILayout.BeginScrollView(_scroll, GUILayout.Height(180));
            GUILayout.TextArea(_log.Dump());
            GUILayout.EndScrollView();

            GUILayout.EndArea();
        }

        private void Initialize()
        {
            if (initialized)
            {
                _log.Log("Initialize: already initialized");
                ApplyConsent();
                return;
            }
            SdkBridge.Initialize(appId, testMode);
            initialized = true;
            _log.Log($"Initialize OK (testMode={testMode})");
            ApplyConsent();
        }

        private void ApplyConsent()
        {
            var payload = new ConsentPayload
            {
                gdprApplies = gdprApplies,
                tcfString = gdprApplies ? (tcfString ?? "TCF_TEST_STRING") : null,
                ccpaOptOut = ccpaOptOut,
                coppa = coppa,
                testMode = testMode
            };
            SdkBridge.SetConsent(payload);
            _log.Log($"Consent updated: GDPR={gdprApplies} CCPA={ccpaOptOut} COPPA={coppa} testMode={testMode}");
        }

        // Event handlers
        private void HandleAdLoaded(string placementId, AdFormat fmt)
        {
            if (fmt == AdFormat.Interstitial) interstitialLoaded = true;
            if (fmt == AdFormat.Rewarded) rewardedLoaded = true;
            _log.Log($"onAdLoaded({placementId},{fmt})");
        }
        private void HandleAdFailedToLoad(string placementId, AdFormat fmt, AdError err, string msg)
        {
            lastError = $"{err.ToMessage()}: {msg}";
            _log.Log($"onAdFailedToLoad({placementId},{fmt}): {lastError}");
        }
        private void HandleAdShown(string placementId, AdFormat fmt)
        {
            _log.Log($"onAdShown({placementId},{fmt})");
        }
        private void HandleAdFailedToShow(string placementId, AdFormat fmt, AdError err, string msg)
        {
            lastError = $"{err.ToMessage()}: {msg}";
            presenting = false;
            _log.Log($"onAdFailedToShow({placementId},{fmt}): {lastError}");
        }
        private void HandleAdClosed(string placementId, AdFormat fmt)
        {
            presenting = false;
            if (fmt == AdFormat.Interstitial) interstitialLoaded = false;
            if (fmt == AdFormat.Rewarded) rewardedLoaded = false;
            _log.Log($"onAdClosed({placementId},{fmt})");
        }
        private void HandleUserEarnedReward(string placementId, double amount)
        {
            _log.Log($"onUserEarnedReward({placementId}):{amount}");
        }

        // JSON config
        [Serializable]
        private class Config
        {
            public string appId;
            public Placements placements;
            public Consent consent;
            public bool testMode = true;
        }
        [Serializable]
        private class Placements { public string interstitialA; public string rewardedA; public string bannerA; }
        [Serializable]
        private class Consent { public bool gdpr; public bool ccpa; public bool coppa; }
    }

    // Auto-bootstrap a GameObject with the controller if none present
    public static class SandboxBootstrap
    {
        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.AfterSceneLoad)]
        static void EnsureController()
        {
            if (GameObject.FindObjectOfType<SandboxController>() != null) return;
            var go = new GameObject("ApexSandboxUnity");
            go.AddComponent<SandboxController>();
            GameObject.DontDestroyOnLoad(go);
        }
    }
}
