using System;
using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Metadata describing an adapter the publisher enables inside BYO mode.
    /// </summary>
    public sealed class NetworkAdapterDescriptor
    {
        public NetworkAdapterDescriptor(string name, bool supportsS2S, IReadOnlyCollection<string>? requiredCredentialKeys = null)
        {
            if (string.IsNullOrWhiteSpace(name))
            {
                throw new ArgumentException("Adapter name must not be empty", nameof(name));
            }

            Name = name.Trim();
            SupportsS2S = supportsS2S;
            RequiredCredentialKeys = requiredCredentialKeys ?? Array.Empty<string>();
        }

        public string Name { get; }
        public bool SupportsS2S { get; }
        public IReadOnlyCollection<string> RequiredCredentialKeys { get; }
    }
}
