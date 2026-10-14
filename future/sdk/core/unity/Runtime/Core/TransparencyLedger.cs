using System;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;

namespace Apex.Mediation.Core
{
    internal sealed class TransparencyLedger
    {
        private readonly List<TransparencyProof> _proofs = new();
        private readonly object _gate = new();
        private string _lastHash = "0000";

        public void Record(TelemetryTrace trace)
        {
            lock (_gate)
            {
                var payload = $"{trace.Timestamp:o}|{trace.PlacementId}|{trace.Adapter}|{trace.Outcome}|{trace.Latency.TotalMilliseconds}";
                var hash = Hash(payload + _lastHash);
                var proof = new TransparencyProof(trace.Timestamp, trace.PlacementId, trace.Adapter, trace.Outcome, hash, _lastHash);
                _proofs.Add(proof);
                _lastHash = hash;
            }
        }

        public IReadOnlyList<TransparencyProof> Snapshot()
        {
            lock (_gate)
            {
                return _proofs.ToArray();
            }
        }

        private static string Hash(string value)
        {
            using var sha = SHA256.Create();
            var bytes = sha.ComputeHash(Encoding.UTF8.GetBytes(value));
            return Convert.ToBase64String(bytes);
        }
    }
}
