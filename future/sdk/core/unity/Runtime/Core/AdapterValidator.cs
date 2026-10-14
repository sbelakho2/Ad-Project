using System.Collections.Generic;
using System.Linq;
using Apex.Mediation.Adapters;
using Apex.Mediation.Internal;

namespace Apex.Mediation.Core
{
    internal sealed class AdapterValidator
    {
        private readonly MediationSDK _sdk;

        public AdapterValidator(MediationSDK sdk)
        {
            _sdk = sdk;
        }

        public void Validate(string network, IReadOnlyDictionary<string, object?> credentials, System.Action<AdapterValidationResult> callback)
        {
            if (!AdapterCatalog.TryGet(network, out var metadata))
            {
                callback?.Invoke(new AdapterValidationResult(network, false, "Unknown adapter"));
                return;
            }

            var missing = metadata.RequiredCredentialKeys.Where(key => !credentials.ContainsKey(key) || credentials[key] == null || string.IsNullOrEmpty(credentials[key]?.ToString())).ToList();
            if (missing.Any())
            {
                callback?.Invoke(new AdapterValidationResult(network, false, "Missing credentials", missing));
                return;
            }

            if (_sdk.PlatformBridge.SupportsValidation)
            {
                _sdk.PlatformBridge.ValidateAdapter(network, credentials, (success, message) =>
                {
                    callback?.Invoke(new AdapterValidationResult(network, success, message ?? (success ? "Validated" : "Validation failed")));
                });
            }
            else
            {
                Logger.Log("Platform bridge does not support validation, returning local success");
                callback?.Invoke(new AdapterValidationResult(network, true, "Locally validated"));
            }
        }
    }
}
