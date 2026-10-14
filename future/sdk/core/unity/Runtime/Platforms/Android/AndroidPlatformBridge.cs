#if UNITY_ANDROID
using System;
using System.Collections.Generic;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using Apex.Mediation.Internal;
using UnityEngine;

namespace Apex.Mediation.Platforms.Android
{
    internal sealed class AndroidPlatformBridge : IPlatformBridge
    {
        private AndroidJavaObject? _sdk;
        public string PlatformName => "Android";
        public bool SupportsValidation => true;

        public void Initialize(ApexConfig config, ConsentOptions consent, CredentialStore credentialStore)
        {
            using var sdkClass = new AndroidJavaClass("com.apex.mediation.MediationSDK");
            _sdk = sdkClass.CallStatic<AndroidJavaObject>("getInstance");
            _sdk?.Call("initialize", JsonUtility.ToJson(config), JsonUtility.ToJson(consent));
            Logger.Log("Android bridge initialized");
        }

        public void LoadInterstitial(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            _sdk?.Call("loadInterstitial", placementId, Serialize(extras), new AndroidLoadCallback(callback));
        }

        public void ShowInterstitial(string placementId, Action<bool, string?> callback)
        {
            _sdk?.Call("showInterstitial", placementId, new AndroidLoadCallback(callback));
        }

        public void LoadRewarded(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            _sdk?.Call("loadRewarded", placementId, Serialize(extras), new AndroidLoadCallback(callback));
        }

        public void ShowRewarded(string placementId, Action<bool, string?> callback)
        {
            _sdk?.Call("showRewarded", placementId, new AndroidLoadCallback(callback));
        }

        public void LoadBanner(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            _sdk?.Call("loadBanner", placementId, Serialize(extras), new AndroidLoadCallback(callback));
        }

        public void AttachBanner(string placementId, IntPtr viewHandle)
        {
            _sdk?.Call("attachBanner", placementId, (int)viewHandle);
        }

        public void DestroyBanner(string placementId)
        {
            _sdk?.Call("destroyBanner", placementId);
        }

        public void UpdateConsent(ConsentOptions consent)
        {
            _sdk?.Call("setConsent", JsonUtility.ToJson(consent));
        }

        public void Shutdown()
        {
            _sdk?.Call("shutdown");
            _sdk = null;
        }

        public void ValidateAdapter(string network, IReadOnlyDictionary<string, object?> credentials, Action<bool, string?> callback)
        {
            _sdk?.Call("validateAdapter", network, Serialize(new Dictionary<string, object?>(credentials)), new AndroidLoadCallback(callback));
        }

        private static string Serialize(Dictionary<string, object?> extras)
        {
            return JsonUtility.ToJson(new AndroidMapWrapper(extras));
        }

        private sealed class AndroidMapWrapper
        {
            public Dictionary<string, object?> extras;
            public AndroidMapWrapper(Dictionary<string, object?> map) => extras = map;
        }

        private sealed class AndroidLoadCallback : AndroidJavaProxy
        {
            private readonly Action<bool, string?> _callback;
            public AndroidLoadCallback(Action<bool, string?> callback) : base("com.apex.mediation.LoadCallback")
            {
                _callback = callback;
            }

            public void onResult(bool success, string error)
            {
                _callback?.Invoke(success, error);
            }
        }
    }
}
#endif
