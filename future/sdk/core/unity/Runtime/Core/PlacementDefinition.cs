using System;

namespace Apex.Mediation.Core
{
    [Serializable]
    public sealed class PlacementDefinition
    {
        public PlacementDefinition(string placementId, PlacementFormat format, double floorCpm = 0)
        {
            PlacementId = placementId;
            Format = format;
            FloorCpm = floorCpm;
        }

        public string PlacementId { get; }
        public PlacementFormat Format { get; }
        public double FloorCpm { get; }
    }
}
