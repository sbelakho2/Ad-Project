using System;
using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    public sealed class TelemetryTrace
    {
        public TelemetryTrace(string placementId, string adapter, string outcome, TimeSpan latency, IReadOnlyDictionary<string, object?> extras)
        {
            PlacementId = placementId;
            Adapter = adapter;
            Outcome = outcome;
            Latency = latency;
            Extras = extras;
            Timestamp = DateTime.UtcNow;
        }

        public string PlacementId { get; }
        public string Adapter { get; }
        public string Outcome { get; }
        public TimeSpan Latency { get; }
        public IReadOnlyDictionary<string, object?> Extras { get; }
        public DateTime Timestamp { get; }
    }
}
