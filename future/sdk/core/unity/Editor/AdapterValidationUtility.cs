using System.Collections.Generic;
using System.Linq;
using Apex.Mediation;
using Apex.Mediation.Adapters;
using Apex.Mediation.Core;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    internal static class AdapterValidationUtility
    {
        public static AdapterValidationResult Validate(string network)
        {
            var settings = AdapterCredentialSettings.LoadOrCreate();
            if (!settings.TryGet(network, out var entry))
            {
                return new AdapterValidationResult(network, false, "No credentials saved");
            }

            var payload = entry.AsDictionary().ToDictionary(kvp => kvp.Key, kvp => (object?)kvp.Value);

            var offline = ValidateOffline(network, payload);
            if (Application.isPlaying && ApexMediation.IsInitialized)
            {
                ApexMediation.ValidateAdapter(network, payload, result =>
                {
                    Debug.Log($"Runtime validation {network}: {result.Message}");
                });
            }

            return offline;
        }

        private static AdapterValidationResult ValidateOffline(string network, IReadOnlyDictionary<string, object?> payload)
        {
            if (!AdapterCatalog.TryGet(network, out var metadata))
            {
                return new AdapterValidationResult(network, true, "Unknown adapter — assumed valid");
            }

            var missing = metadata.RequiredCredentialKeys
                .Where(key => !payload.ContainsKey(key) || string.IsNullOrEmpty(payload[key]?.ToString()))
                .ToList();

            if (missing.Any())
            {
                return new AdapterValidationResult(network, false, "Missing credentials", missing);
            }

            return new AdapterValidationResult(network, true, "Keys present");
        }
    }
}