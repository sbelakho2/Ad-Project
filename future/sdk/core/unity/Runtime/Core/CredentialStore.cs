using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using Apex.Mediation.Internal;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Runtime credential storage kept entirely within the host app boundary.
    /// </summary>
    internal sealed class CredentialStore
    {
        private readonly ConcurrentDictionary<string, IReadOnlyDictionary<string, object?>> _runtimeConfigs = new(StringComparer.OrdinalIgnoreCase);
        private readonly IAdapterConfigProvider _provider;

        public CredentialStore(IAdapterConfigProvider provider)
        {
            _provider = provider ?? NullAdapterConfigProvider.Instance;
        }

        public void SetRuntimeConfig(string network, IReadOnlyDictionary<string, object?> config)
        {
            if (string.IsNullOrWhiteSpace(network))
            {
                throw new ArgumentException("Network must be provided", nameof(network));
            }

            _runtimeConfigs[network.Trim()] = config;
        }

        public IReadOnlyDictionary<string, object?>? Resolve(string network)
        {
            if (string.IsNullOrWhiteSpace(network))
            {
                return null;
            }

            if (_runtimeConfigs.TryGetValue(network, out var config))
            {
                return config;
            }

            return _provider.GetConfig(network);
        }

        public bool HasS2SCreds(IEnumerable<NetworkAdapterDescriptor> adapters)
        {
            foreach (var adapter in adapters)
            {
                var config = Resolve(adapter.Name);
                if (config == null)
                {
                    Logger.LogDebug($"No runtime config for {adapter.Name}");
                    return false;
                }

                foreach (var key in adapter.RequiredCredentialKeys)
                {
                    if (!config.ContainsKey(key))
                    {
                        Logger.LogDebug($"Missing credential key {key} for {adapter.Name}");
                        return false;
                    }
                }
            }

            return true;
        }
    }
}
