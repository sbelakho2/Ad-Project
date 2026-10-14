using System;
using System.Collections.Generic;
using System.Reflection;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using Apex.Mediation.Platforms;
using Xunit;

[Collection("MediationSDK")]
public sealed class MediationSdkShowTests
{
    private const string PlacementId = "unity_test_interstitial";

    public MediationSdkShowTests()
    {
        ResetSdk();
    }

    [Fact]
    public void ShowInterstitial_FiresCallbackOnce_WhenPlatformReportsMultipleResults()
    {
        var duplicateBridge = new DuplicateCallbackBridge();
        ReplaceBridge(duplicateBridge);

        var callbackCount = 0;
        bool? finalSuccess = null;
        string? finalError = null;

        MediationSDK.Instance.ShowInterstitial(PlacementId, (success, error) =>
        {
            callbackCount++;
            finalSuccess = success;
            finalError = error;
        });

        Assert.Equal(1, callbackCount);
        Assert.False(finalSuccess);
        Assert.Equal("first", finalError);
        Assert.Equal(2, duplicateBridge.InvocationCount);
    }

    private static void ResetSdk()
    {
        var sdk = MediationSDK.Instance;
        sdk.Shutdown();

        var config = new ApexConfig
        {
            AppId = "unity-test-app"
        };
        config.DefinePlacement(PlacementId, PlacementFormat.Interstitial);

        sdk.Initialize(config, _ => { });
        sdk.LoadInterstitial(PlacementId, (_, _) => { });
    }

    private static void ReplaceBridge(IPlatformBridge bridge)
    {
        var field = typeof(MediationSDK).GetField("_bridge", BindingFlags.Instance | BindingFlags.NonPublic);
        field!.SetValue(MediationSDK.Instance, bridge);
    }

    private sealed class DuplicateCallbackBridge : IPlatformBridge
    {
        public int InvocationCount { get; private set; }
        public string PlatformName => "Duplicate";
        public bool SupportsValidation => false;

        public void Initialize(ApexConfig config, ConsentOptions consent, CredentialStore credentialStore)
        {
        }

        public void LoadInterstitial(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void ShowInterstitial(string placementId, Action<bool, string?> callback)
        {
            InvocationCount++;
            callback(false, "first");
            InvocationCount++;
            callback(true, null);
        }

        public void LoadRewarded(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
        {
            callback?.Invoke(true, null);
        }

        public void ShowRewarded(string placementId, Action<bool, string?> callback)
        {
            callback(true, null);
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
            callback?.Invoke(true, null);
        }
    }
}
