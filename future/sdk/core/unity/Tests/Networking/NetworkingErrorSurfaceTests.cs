using System;
using System.Collections.Generic;
using System.Reflection;
using Apex.Mediation;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using Apex.Mediation.Platforms;
using Xunit;

namespace Apex.Mediation.Tests.Networking
{
    [Collection("MediationSDK")]
    public sealed class NetworkingErrorSurfaceTests : IDisposable
    {
        private const string PlacementId = "unity_network_test";
        private readonly RecordingBridge _bridge = new();

        public NetworkingErrorSurfaceTests()
        {
            ResetSdk();
            ReplaceBridge(_bridge);
        }

        public void Dispose()
        {
            MediationSDK.Instance.Shutdown();
        }

        [Fact]
        public void LoadInterstitial_SurfacesNoFillErrors()
        {
            _bridge.EnqueueResponse(false, "no_fill");

            AdEventArgs? observed = null;
            void Handler(AdEventArgs args) => observed = args;
            ApexMediation.OnAdEvent += Handler;

            try
            {
                var result = InvokeLoad();
                Assert.False(result.Success);
                Assert.Equal("no_fill", result.Error);
                Assert.NotNull(observed);
                Assert.Equal(AdEventType.FailedToLoad, observed!.Type);
                Assert.Equal("no_fill", observed.Message);
            }
            finally
            {
                ApexMediation.OnAdEvent -= Handler;
            }
        }

        [Fact]
        public void LoadInterstitial_SurfacesTimeouts()
        {
            _bridge.EnqueueResponse(false, "timeout");

            AdEventArgs? observed = null;
            void Handler(AdEventArgs args) => observed = args;
            ApexMediation.OnAdEvent += Handler;

            try
            {
                var result = InvokeLoad();
                Assert.False(result.Success);
                Assert.Equal("timeout", result.Error);
                Assert.NotNull(observed);
                Assert.Equal("timeout", observed!.Message);
            }
            finally
            {
                ApexMediation.OnAdEvent -= Handler;
            }
        }

        [Fact]
        public void LoadInterstitial_SurfacesRateLimitedErrorsWithRetryMetadata()
        {
            _bridge.EnqueueResponse(false, "rate_limited: retry after 2s");

            AdEventArgs? observed = null;
            void Handler(AdEventArgs args) => observed = args;
            ApexMediation.OnAdEvent += Handler;

            try
            {
                var result = InvokeLoad();
                Assert.False(result.Success);
                Assert.Equal("rate_limited: retry after 2s", result.Error);
                Assert.NotNull(observed);
                Assert.Equal("rate_limited: retry after 2s", observed!.Message);
            }
            finally
            {
                ApexMediation.OnAdEvent -= Handler;
            }
        }

        [Fact]
        public void LoadInterstitial_AllowsRetriesAfterServerFailures()
        {
            _bridge.EnqueueResponse(false, "status_5xx: 503 after retries");
            _bridge.EnqueueResponse(true, null);

            var first = InvokeLoad();
            Assert.False(first.Success);
            Assert.Equal("status_5xx: 503 after retries", first.Error);

            var second = InvokeLoad();
            Assert.True(second.Success);
            Assert.Null(second.Error);
        }

        private static void ResetSdk()
        {
            var sdk = MediationSDK.Instance;
            sdk.Shutdown();

            var config = new ApexConfig
            {
                AppId = "unity-network-tests"
            };
            config.DefinePlacement(PlacementId, PlacementFormat.Interstitial);
            config.EnableAdapter(new NetworkAdapterDescriptor("mock", supportsS2S: true));

            sdk.Initialize(config, _ => { });
        }

        private static void ReplaceBridge(IPlatformBridge bridge)
        {
            var field = typeof(MediationSDK).GetField("_bridge", BindingFlags.Instance | BindingFlags.NonPublic);
            field!.SetValue(MediationSDK.Instance, bridge);
        }

        private static LoadResult InvokeLoad()
        {
            bool? success = null;
            string? error = null;
            ApexMediation.LoadInterstitial(PlacementId, (s, e) =>
            {
                success = s;
                error = e;
            });

            if (!success.HasValue)
            {
                throw new InvalidOperationException("Load callback was not invoked");
            }

            return new LoadResult(success.Value, error);
        }

        private sealed record LoadResult(bool Success, string? Error);

        private sealed class RecordingBridge : IPlatformBridge
        {
            private readonly Queue<(bool Success, string? Error)> _loadResponses = new();

            public string PlatformName => "TestBridge";
            public bool SupportsValidation => false;

            public void EnqueueResponse(bool success, string? error)
            {
                _loadResponses.Enqueue((success, error));
            }

            public void Initialize(ApexConfig config, ConsentOptions consent, CredentialStore credentialStore)
            {
            }

            public void LoadInterstitial(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
            {
                var response = _loadResponses.Count > 0 ? _loadResponses.Dequeue() : (Success: true, Error: (string?)null);
                callback?.Invoke(response.Success, response.Error);
            }

            public void ShowInterstitial(string placementId, Action<bool, string?> callback)
            {
                callback(true, null);
            }

            public void LoadRewarded(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
            {
                callback(true, null);
            }

            public void ShowRewarded(string placementId, Action<bool, string?> callback)
            {
                callback(true, null);
            }

            public void LoadBanner(string placementId, Dictionary<string, object?> extras, Action<bool, string?> callback)
            {
                callback(true, null);
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
                callback(true, null);
            }
        }
    }
}
