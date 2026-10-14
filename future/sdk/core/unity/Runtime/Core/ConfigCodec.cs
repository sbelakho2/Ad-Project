using System;
using System.Collections.Generic;
#if UNITY_2020_3_OR_NEWER
using UnityEngine;
#else
using System.Text.Json;
#endif

namespace Apex.Mediation.Core
{
    public static class ConfigCodec
    {
#if !UNITY_2020_3_OR_NEWER
        private static readonly JsonSerializerOptions JsonOptions = new()
        {
            WriteIndented = true,
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            IncludeFields = true,
            PropertyNameCaseInsensitive = true
        };
#endif

        public static string Export(ApexConfig config, string signingKey)
        {
            if (config == null)
            {
                throw new ArgumentNullException(nameof(config));
            }

            var document = ConfigDocument.From(config);
            var payload = Serialize(document.WithoutSignature());
            document.signature = ConfigSignature.Sign(payload, signingKey);
            return Serialize(document);
        }

        public static ApexConfig Import(string json, string signingKey)
        {
            if (string.IsNullOrWhiteSpace(json))
            {
                throw new ArgumentException("JSON must be provided", nameof(json));
            }

            var document = Deserialize(json);
            if (document == null)
            {
                throw new InvalidOperationException("Invalid config document");
            }

            var payload = Serialize(document.WithoutSignature());
            if (!ConfigSignature.Verify(payload, signingKey, document.signature))
            {
                throw new InvalidOperationException("Signature mismatch");
            }

            return document.ToConfig();
        }

        private static string Serialize(ConfigDocument document)
        {
#if UNITY_2020_3_OR_NEWER
            return JsonUtility.ToJson(document, prettyPrint: true);
#else
        return JsonSerializer.Serialize(document, JsonOptions);
#endif
        }

        private static ConfigDocument? Deserialize(string json)
        {
#if UNITY_2020_3_OR_NEWER
            return JsonUtility.FromJson<ConfigDocument>(json);
#else
        return JsonSerializer.Deserialize<ConfigDocument>(json, JsonOptions);
#endif
        }

        [Serializable]
        private sealed class ConfigDocument
        {
            public int version = 1;
            public string appId = string.Empty;
            public string mode = SdkMode.BYO.ToString();
            public bool enableAutoConsentRead = true;
            public bool enableViewability = true;
            public bool enableOmSdk = false;
            public bool enableTelemetry = true;
            public double renderTimeoutSeconds = 3.5d;
            public double adCacheTtlMinutes = 30;
            public AdapterDto[] adapters = Array.Empty<AdapterDto>();
            public PlacementDto[] placements = Array.Empty<PlacementDto>();
            public string signature = string.Empty;

            public static ConfigDocument From(ApexConfig config)
            {
                var doc = new ConfigDocument
                {
                    appId = config.AppId,
                    mode = config.Mode.ToString(),
                    enableAutoConsentRead = config.EnableAutoConsentRead,
                    enableViewability = config.EnableViewability,
                    enableOmSdk = config.EnableOmSdk,
                    enableTelemetry = config.EnableTelemetry,
                    renderTimeoutSeconds = config.RenderTimeoutSeconds,
                    adCacheTtlMinutes = config.AdCacheTtl.TotalMinutes,
                    adapters = new AdapterDto[config.EnabledAdapters.Count],
                    placements = new PlacementDto[config.Placements.Count]
                };

                for (var i = 0; i < config.EnabledAdapters.Count; i++)
                {
                    var adapter = config.EnabledAdapters[i];
                    doc.adapters[i] = new AdapterDto
                    {
                        name = adapter.Name,
                        supportsS2S = adapter.SupportsS2S,
                        requiredCredentialKeys = adapter.RequiredCredentialKeys is string[] arr
                            ? arr
                            : new List<string>(adapter.RequiredCredentialKeys).ToArray()
                    };
                }

                for (var i = 0; i < config.Placements.Count; i++)
                {
                    var placement = config.Placements[i];
                    doc.placements[i] = new PlacementDto
                    {
                        placementId = placement.PlacementId,
                        format = placement.Format.ToString(),
                        floorCpm = placement.FloorCpm
                    };
                }

                return doc;
            }

            public ApexConfig ToConfig()
            {
                var config = new ApexConfig
                {
                    AppId = appId,
                    Mode = Enum.TryParse(mode, true, out SdkMode parsed) ? parsed : SdkMode.BYO,
                    EnableAutoConsentRead = enableAutoConsentRead,
                    EnableViewability = enableViewability,
                    EnableOmSdk = enableOmSdk,
                    EnableTelemetry = enableTelemetry,
                    RenderTimeoutSeconds = renderTimeoutSeconds,
                    AdCacheTtl = TimeSpan.FromMinutes(adCacheTtlMinutes)
                };

                if (adapters != null)
                {
                    foreach (var adapter in adapters)
                    {
                        config.EnableAdapter(new NetworkAdapterDescriptor(adapter.name, adapter.supportsS2S, adapter.requiredCredentialKeys ?? Array.Empty<string>()));
                    }
                }

                if (placements != null)
                {
                    foreach (var placement in placements)
                    {
                        var format = Enum.TryParse(placement.format, true, out PlacementFormat fmt)
                            ? fmt
                            : PlacementFormat.Interstitial;
                        config.DefinePlacement(placement.placementId, format, placement.floorCpm);
                    }
                }

                return config;
            }

            public ConfigDocument WithoutSignature()
            {
                return new ConfigDocument
                {
                    version = version,
                    appId = appId,
                    mode = mode,
                    enableAutoConsentRead = enableAutoConsentRead,
                    enableViewability = enableViewability,
                    enableOmSdk = enableOmSdk,
                    enableTelemetry = enableTelemetry,
                    renderTimeoutSeconds = renderTimeoutSeconds,
                    adCacheTtlMinutes = adCacheTtlMinutes,
                    adapters = adapters,
                    placements = placements,
                    signature = string.Empty
                };
            }
        }

        [Serializable]
        private sealed class AdapterDto
        {
            public string name = string.Empty;
            public bool supportsS2S;
            public string[] requiredCredentialKeys = Array.Empty<string>();
        }

        [Serializable]
        private sealed class PlacementDto
        {
            public string placementId = string.Empty;
            public string format = PlacementFormat.Interstitial.ToString();
            public double floorCpm;
        }

    }
}
