using System;
using System.Collections.Generic;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;

namespace Apex.Mediation.Platforms
{
    internal interface IPlatformBridge
    {
        string PlatformName { get; }
        bool SupportsValidation { get; }
        void Initialize(ApexConfig config, ConsentOptions consent, CredentialStore credentialStore);
        void LoadInterstitial(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback);
        void ShowInterstitial(string placementId, Action<bool, string?> callback);
        void LoadRewarded(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback);
        void ShowRewarded(string placementId, Action<bool, string?> callback);
        void LoadBanner(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback);
        void AttachBanner(string placementId, IntPtr viewHandle);
        void DestroyBanner(string placementId);
        void UpdateConsent(ConsentOptions consent);
        void Shutdown();
        void ValidateAdapter(string network, IReadOnlyDictionary<string, object?> credentials, Action<bool, string?> callback);
    }
}
