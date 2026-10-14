#if UNITY_IOS
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using UnityEngine;

namespace Apex.Mediation.Platforms.iOS
{
    internal sealed class IosPlatformBridge : IPlatformBridge
    {
        public string PlatformName => "iOS";
        public bool SupportsValidation => true;

        public void Initialize(ApexConfig config, ConsentOptions consent, CredentialStore credentialStore)
        {
            ApexMediationInitialize(JsonUtility.ToJson(config), JsonUtility.ToJson(consent));
        }

        public void LoadInterstitial(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            ApexMediationLoadInterstitial(placementId, Serialize(extras), CallbackRouter.Alloc(callback));
        }

        public void ShowInterstitial(string placementId, Action<bool, string?> callback)
        {
            ApexMediationShowInterstitial(placementId, CallbackRouter.Alloc(callback));
        }

        public void LoadRewarded(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            ApexMediationLoadRewarded(placementId, Serialize(extras), CallbackRouter.Alloc(callback));
        }

        public void ShowRewarded(string placementId, Action<bool, string?> callback)
        {
            ApexMediationShowRewarded(placementId, CallbackRouter.Alloc(callback));
        }

        public void LoadBanner(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            ApexMediationLoadBanner(placementId, Serialize(extras), CallbackRouter.Alloc(callback));
        }

        public void AttachBanner(string placementId, IntPtr viewHandle)
        {
            ApexMediationAttachBanner(placementId, viewHandle);
        }

        public void DestroyBanner(string placementId)
        {
            ApexMediationDestroyBanner(placementId);
        }

        public void UpdateConsent(ConsentOptions consent)
        {
            ApexMediationSetConsent(JsonUtility.ToJson(consent));
        }

        public void Shutdown()
        {
            ApexMediationShutdown();
        }

        public void ValidateAdapter(string network, IReadOnlyDictionary<string, object?> credentials, Action<bool, string?> callback)
        {
            ApexMediationValidateAdapter(network, Serialize(new Dictionary<string, object?>(credentials)), CallbackRouter.Alloc((success, message) => callback?.Invoke(success, message)));
        }

        private static string Serialize(Dictionary<string, object?> extras)
        {
            return JsonUtility.ToJson(new Wrapper { extras = extras });
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct Wrapper
        {
            public Dictionary<string, object?> extras;
        }

        private static class CallbackRouter
        {
            private static readonly Dictionary<IntPtr, Action<bool, string?>> Map = new();

            public static IntPtr Alloc(Action<bool, string?> callback)
            {
                var handle = (IntPtr)Map.Count + 1;
                Map[handle] = callback;
                return handle;
            }

            [AOT.MonoPInvokeCallback(typeof(NativeCallback))]
            public static void Invoke(IntPtr handle, bool success, string error)
            {
                if (Map.TryGetValue(handle, out var callback))
                {
                    callback?.Invoke(success, error);
                    Map.Remove(handle);
                }
            }
        }

        private delegate void NativeCallback(IntPtr handle, bool success, string error);

        [DllImport("__Internal")]
        private static extern void ApexMediationInitialize(string configJson, string consentJson);

        [DllImport("__Internal")]
        private static extern void ApexMediationLoadInterstitial(string placementId, string extrasJson, IntPtr callbackHandle);

        [DllImport("__Internal")]
        private static extern void ApexMediationShowInterstitial(string placementId, IntPtr callbackHandle);

        [DllImport("__Internal")]
        private static extern void ApexMediationLoadRewarded(string placementId, string extrasJson, IntPtr callbackHandle);

        [DllImport("__Internal")]
        private static extern void ApexMediationShowRewarded(string placementId, IntPtr callbackHandle);

        [DllImport("__Internal")]
        private static extern void ApexMediationLoadBanner(string placementId, string extrasJson, IntPtr callbackHandle);

        [DllImport("__Internal")]
        private static extern void ApexMediationAttachBanner(string placementId, IntPtr viewHandle);

        [DllImport("__Internal")]
        private static extern void ApexMediationDestroyBanner(string placementId);

        [DllImport("__Internal")]
        private static extern void ApexMediationSetConsent(string consentJson);

        [DllImport("__Internal")]
        private static extern void ApexMediationShutdown();

        [DllImport("__Internal")]
        private static extern void ApexMediationValidateAdapter(string network, string credentialsJson, IntPtr callbackHandle);
    }
}
#endif
