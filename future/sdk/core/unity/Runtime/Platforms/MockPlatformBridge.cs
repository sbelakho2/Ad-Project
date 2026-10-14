using System;
using System.Collections.Generic;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using Apex.Mediation.Internal;

namespace Apex.Mediation.Platforms
{
    internal sealed class MockPlatformBridge : IPlatformBridge
    {
        public string PlatformName => "Mock";
        public bool SupportsValidation => true;

        public void Initialize(ApexConfig config, ConsentOptions consent, CredentialStore credentialStore)
        {
            Logger.Log("Mock bridge initialized");
        }

        public void LoadInterstitial(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void ShowInterstitial(string placementId, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void LoadRewarded(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void ShowRewarded(string placementId, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void LoadBanner(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void AttachBanner(string placementId, IntPtr viewHandle)
        {
        }

        public void DestroyBanner(string placementId)
        {
        }

        public void UpdateConsent(ConsentOptions consent)
        {
        }

        public void Shutdown()
        {
        }

        public void ValidateAdapter(string network, IReadOnlyDictionary<string, object?> credentials, Action<bool, string?> callback)
        {
            var success = credentials != null && credentials.Count > 0;
            callback?.Invoke(success, success ? null : "Missing credentials");
        }
    }
}
