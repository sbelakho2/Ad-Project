using System;

namespace Apex.Mediation.Core
{
    public sealed class TransparencyProof
    {
        public TransparencyProof(DateTime timestamp, string placementId, string adapter, string outcome, string hash, string previousHash)
        {
            Timestamp = timestamp;
            PlacementId = placementId;
            Adapter = adapter;
            Outcome = outcome;
            Hash = hash;
            PreviousHash = previousHash;
        }

        public DateTime Timestamp { get; }
        public string PlacementId { get; }
        public string Adapter { get; }
        public string Outcome { get; }
        public string Hash { get; }
        public string PreviousHash { get; }
    }
}
