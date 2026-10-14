#if UNITY_2020_3_OR_NEWER
using System;
using System.Collections.Generic;
using Apex.Mediation.Consent;
using Apex.Mediation.Core;
using Apex.Mediation.Internal;
using UnityEngine;

namespace Apex.Mediation
{
    /// <summary>
    /// Drop-in MonoBehaviour that initializes ApexMediation from a signed config JSON.
    /// Ensures a single entry point for SDK usage in every scene.
    /// </summary>
    [DisallowMultipleComponent]
    [DefaultExecutionOrder(-800)]
    public sealed class ApexMediationEntryPoint : MonoBehaviour
    {
        [Header("Config-as-Code")]
        [SerializeField] private TextAsset signedConfigJson = default!;
        [SerializeField] private string signingKey = string.Empty;

        [Header("Behavior")]
        [SerializeField] private bool initializeOnAwake = true;
        [SerializeField] private bool attachDebuggerOverlayInEditor = true;
        [SerializeField] private bool logAdEventsToConsole = true;

        [Header("Runtime Credentials (optional)")]
        [SerializeField] private AdapterCredential[] adapterCredentials = Array.Empty<AdapterCredential>();

        private bool _initializedFromComponent;

        private void Awake()
        {
            if (initializeOnAwake)
            {
                InitializeIfNeeded();
            }
        }

        private void OnDestroy()
        {
            if (_initializedFromComponent && logAdEventsToConsole)
            {
                ApexMediation.OnAdEvent -= HandleAdEvent;
            }
        }

        /// <summary>
        /// Initializes the SDK using the provided signed config (if not already initialized).
        /// </summary>
        public void InitializeIfNeeded()
        {
            if (ApexMediation.IsInitialized)
            {
                return;
            }

            if (signedConfigJson == null || string.IsNullOrWhiteSpace(signedConfigJson.text))
            {
                Logger.LogError("ApexMediationEntryPoint is missing a signed config TextAsset.");
                return;
            }

            if (string.IsNullOrWhiteSpace(signingKey))
            {
                Logger.LogError("ApexMediationEntryPoint requires a signing key to verify the config payload.");
                return;
            }

            try
            {
                var config = ConfigCodec.Import(signedConfigJson.text, signingKey);
                var provider = InspectorAdapterConfigProvider.Create(adapterCredentials);
                if (provider != null)
                {
                    config.AdapterConfigProvider = provider;
                }

                ApexMediation.Initialize(config, success =>
                {
                    if (!success)
                    {
                        Logger.LogError("ApexMediation failed to initialize from entry point config.");
                        return;
                    }

                    _initializedFromComponent = true;
                    if (logAdEventsToConsole)
                    {
                        ApexMediation.OnAdEvent -= HandleAdEvent;
                        ApexMediation.OnAdEvent += HandleAdEvent;
                    }

                    if (attachDebuggerOverlayInEditor && Application.isEditor)
                    {
                        MediationDebuggerBootstrap.EnsureOverlay();
                    }
                });
            }
            catch (Exception ex)
            {
                Logger.LogError("Failed to import signed config", ex);
            }
        }

        private static void HandleAdEvent(AdEventArgs args)
        {
            Logger.LogDebug($"AdEvent :: {args.PlacementId} · {args.Adapter} · {args.Type} · {args.Message}");
        }

        [Serializable]
        private sealed class AdapterCredential
        {
            public string network = string.Empty;
            public CredentialKeyValue[] parameters = Array.Empty<CredentialKeyValue>();

            public IReadOnlyDictionary<string, object?> ToDictionary()
            {
                var result = new Dictionary<string, object?>(StringComparer.OrdinalIgnoreCase);
                if (parameters == null)
                {
                    return result;
                }

                foreach (var kv in parameters)
                {
                    if (!string.IsNullOrWhiteSpace(kv.key))
                    {
                        result[kv.key] = kv.value;
                    }
                }

                return result;
            }
        }

        [Serializable]
        private sealed class CredentialKeyValue
        {
            public string key = string.Empty;
            public string value = string.Empty;
        }

        private sealed class InspectorAdapterConfigProvider : IAdapterConfigProvider
        {
            private readonly Dictionary<string, IReadOnlyDictionary<string, object?>> _map;

            private InspectorAdapterConfigProvider(Dictionary<string, IReadOnlyDictionary<string, object?>> map)
            {
                _map = map;
            }

            public static InspectorAdapterConfigProvider? Create(AdapterCredential[] credentials)
            {
                if (credentials == null || credentials.Length == 0)
                {
                    return null;
                }

                var map = new Dictionary<string, IReadOnlyDictionary<string, object?>>(StringComparer.OrdinalIgnoreCase);
                foreach (var credential in credentials)
                {
                    if (string.IsNullOrWhiteSpace(credential?.network))
                    {
                        continue;
                    }

                    map[credential.network] = credential.ToDictionary();
                }

                return map.Count == 0 ? null : new InspectorAdapterConfigProvider(map);
            }

            public IReadOnlyDictionary<string, object?>? GetConfig(string network)
            {
                return _map.TryGetValue(network, out var config) ? config : null;
            }
        }

        private static class MediationDebuggerBootstrap
        {
            public static void EnsureOverlay()
            {
                if (UnityEngine.Object.FindObjectOfType<MediationDebuggerOverlay>() != null)
                {
                    return;
                }

                var go = new GameObject("ApexMediationDebuggerOverlay");
                go.AddComponent<MediationDebuggerOverlay>();
                UnityEngine.Object.DontDestroyOnLoad(go);
            }
        }
    }
}
#endif
