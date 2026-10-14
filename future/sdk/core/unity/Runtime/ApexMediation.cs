using System;
using System.Collections.Generic;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using Apex.Mediation.Internal;

namespace Apex.Mediation
{
    public static class ApexMediation
    {
        public static event Action<AdEventArgs>? OnAdEvent;

        public static bool IsInitialized => MediationSDK.Instance.IsInitialized;

        public static void Initialize(ApexConfig config, Action<bool>? onComplete = null)
        {
            if (config == null)
            {
                throw new ArgumentNullException(nameof(config));
            }

            MediationSDK.Instance.Initialize(config, success =>
            {
                Logger.Log("ApexMediation initialized");
                onComplete?.Invoke(success);
            });
        }

        public static void SetConsent(ConsentOptions consent)
        {
            MediationSDK.Instance.ConsentManager.SetConsent(consent);
            MediationSDK.Instance.PlatformBridge.UpdateConsent(consent);
        }

        public static void SetNetworkConfig(string network, IReadOnlyDictionary<string, object?> config)
        {
            MediationSDK.Instance.SetRuntimeCredential(network, config);
        }

        public static void LoadInterstitial(string placementId, Action<bool, string?>? callback = null)
        {
            MediationSDK.Instance.LoadInterstitial(placementId, (success, error) =>
            {
                callback?.Invoke(success, error);
                RaiseEvent(new AdEventArgs(placementId, success ? AdEventType.Loaded : AdEventType.FailedToLoad, "interstitial", error));
            });
        }

        public static void ShowInterstitial(string placementId, Action<bool, string?>? callback = null)
        {
            MediationSDK.Instance.ShowInterstitial(placementId, (success, error) =>
            {
                callback?.Invoke(success, error);
                RaiseEvent(new AdEventArgs(placementId, success ? AdEventType.Shown : AdEventType.FailedToLoad, "interstitial", error));
            });
        }

        public static void LoadRewarded(string placementId, Action<bool, string?>? callback = null)
        {
            MediationSDK.Instance.LoadRewarded(placementId, (success, error) =>
            {
                callback?.Invoke(success, error);
                RaiseEvent(new AdEventArgs(placementId, success ? AdEventType.Loaded : AdEventType.FailedToLoad, "rewarded", error));
            });
        }

        public static void ShowRewarded(string placementId, Action<bool, string?>? callback = null)
        {
            MediationSDK.Instance.ShowRewarded(placementId, (success, error) =>
            {
                callback?.Invoke(success, error);
                RaiseEvent(new AdEventArgs(placementId, success ? AdEventType.Shown : AdEventType.FailedToLoad, "rewarded", error));
            });
        }

        public static void LoadBanner(string placementId, Action<bool, string?>? callback = null)
        {
            MediationSDK.Instance.LoadBanner(placementId, (success, error) =>
            {
                callback?.Invoke(success, error);
                RaiseEvent(new AdEventArgs(placementId, success ? AdEventType.Loaded : AdEventType.FailedToLoad, "banner", error));
            });
        }

        public static void AttachBanner(string placementId, IntPtr nativeViewHandle)
        {
            MediationSDK.Instance.AttachBanner(placementId, nativeViewHandle);
        }

        public static void DestroyBanner(string placementId)
        {
            MediationSDK.Instance.DestroyBanner(placementId);
            RaiseEvent(new AdEventArgs(placementId, AdEventType.Closed, "banner"));
        }

        public static bool CanShowPersonalizedAds() => MediationSDK.Instance.ConsentManager.CanShowPersonalizedAds();

        public static void Shutdown() => MediationSDK.Instance.Shutdown();

        public static void ValidateAdapter(string network, IReadOnlyDictionary<string, object?> credentials, Action<AdapterValidationResult>? callback = null)
        {
            MediationSDK.Instance.ValidateAdapter(network, credentials, result => callback?.Invoke(result));
        }

        public static IReadOnlyList<TelemetryTrace> GetTelemetryTraces() => MediationSDK.Instance.TelemetrySnapshot;

        public static IReadOnlyList<TransparencyProof> GetTransparencyProofs() => MediationSDK.Instance.TransparencySnapshot;

        private static void RaiseEvent(AdEventArgs args)
        {
            OnAdEvent?.Invoke(args);
        }
    }
}
