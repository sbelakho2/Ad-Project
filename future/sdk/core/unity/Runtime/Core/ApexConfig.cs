using System;
using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Configuration supplied during SDK initialization.
    /// </summary>
    [Serializable]
    public sealed class ApexConfig
    {
        private readonly List<NetworkAdapterDescriptor> _enabledAdapters = new();
        private readonly List<PlacementDefinition> _placements = new();

        public string AppId = string.Empty;
        public SdkMode Mode = SdkMode.BYO;
        public bool EnableAutoConsentRead = true;
        public bool EnableViewability = true;
        public bool EnableOmSdk = false;
        public bool EnableTelemetry = true;
        public double RenderTimeoutSeconds = 3.5d;
        public IAdapterConfigProvider AdapterConfigProvider = NullAdapterConfigProvider.Instance;
        public TimeSpan AdCacheTtl = TimeSpan.FromMinutes(30);

        public IReadOnlyList<NetworkAdapterDescriptor> EnabledAdapters => _enabledAdapters;
        public IReadOnlyList<PlacementDefinition> Placements => _placements;

        public void EnableAdapter(NetworkAdapterDescriptor descriptor)
        {
            if (descriptor == null)
            {
                throw new ArgumentNullException(nameof(descriptor));
            }

            _enabledAdapters.RemoveAll(d => string.Equals(d.Name, descriptor.Name, StringComparison.OrdinalIgnoreCase));
            _enabledAdapters.Add(descriptor);
        }

        public void DefinePlacement(string placementId, PlacementFormat format, double floorCpm = 0)
        {
            if (string.IsNullOrWhiteSpace(placementId))
            {
                throw new ArgumentException("PlacementId must be provided", nameof(placementId));
            }

            _placements.RemoveAll(p => string.Equals(p.PlacementId, placementId, StringComparison.OrdinalIgnoreCase));
            _placements.Add(new PlacementDefinition(placementId, format, floorCpm));
        }
    }
}
