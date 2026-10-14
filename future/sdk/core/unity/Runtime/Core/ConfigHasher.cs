using System;
using System.Collections.Generic;
using System.Linq;
using System.Security.Cryptography;
using System.Text;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Computes deterministic SHA-256 hash of SDK configuration.
    /// Uses sorted JSON serialization to ensure cross-platform parity with server.
    /// Hash format: "v1:&lt;hex-digest&gt;"
    /// </summary>
    public static class ConfigHasher
    {
        /// <summary>
        /// Compute deterministic SHA-256 hash of the current configuration.
        /// </summary>
        /// <param name="config">The SDK configuration to hash</param>
        /// <returns>Configuration hash string in format "v1:&lt;hex&gt;"</returns>
        public static string ComputeHash(ApexConfig config)
        {
            if (config == null)
            {
                throw new ArgumentNullException(nameof(config));
            }

            var canonicalJson = BuildCanonicalConfigJson(config);
            using var sha256 = SHA256.Create();
            var bytes = Encoding.UTF8.GetBytes(canonicalJson);
            var hashBytes = sha256.ComputeHash(bytes);
            var hexHash = BitConverter.ToString(hashBytes).Replace("-", "").ToLowerInvariant();
            return $"v1:{hexHash}";
        }

        /// <summary>
        /// Build canonical JSON representation for hashing.
        /// Keys are sorted alphabetically to ensure deterministic output.
        /// </summary>
        private static string BuildCanonicalConfigJson(ApexConfig config)
        {
            var sb = new StringBuilder();
            sb.Append("{");

            // Add fields in alphabetical order
            var fields = new List<string>();

            // appId
            fields.Add($"\"appId\":\"{EscapeJson(config.AppId)}\"");

            // caps - sorted by placement ID
            var capsBuilder = new StringBuilder();
            capsBuilder.Append("{");
            var sortedPlacements = config.Placements
                .OrderBy(p => p.PlacementId, StringComparer.Ordinal)
                .ToList();
            for (int i = 0; i < sortedPlacements.Count; i++)
            {
                var placement = sortedPlacements[i];
                if (i > 0) capsBuilder.Append(",");
                capsBuilder.Append($"\"{EscapeJson(placement.PlacementId)}\":{{\"daily\":0,\"hourly\":0}}");
            }
            capsBuilder.Append("}");
            fields.Add($"\"caps\":{capsBuilder}");

            // compliance
            fields.Add("\"compliance\":{\"ccpaApplies\":false,\"coppaApplies\":false,\"gdprApplies\":false}");

            // features
            var featuresJson = $"{{\"bannerRefresh\":false,\"bannerRefreshIntervalMs\":0,\"bidding\":true,\"waterfall\":true}}";
            fields.Add($"\"features\":{featuresJson}");

            // floors - sorted by placement ID
            var floorsBuilder = new StringBuilder();
            floorsBuilder.Append("{");
            for (int i = 0; i < sortedPlacements.Count; i++)
            {
                var placement = sortedPlacements[i];
                if (i > 0) floorsBuilder.Append(",");
                floorsBuilder.Append($"\"{EscapeJson(placement.PlacementId)}\":{placement.FloorCpm:F1}");
            }
            floorsBuilder.Append("}");
            fields.Add($"\"floors\":{floorsBuilder}");

            // networks - sorted alphabetically
            var networks = config.EnabledAdapters
                .Select(a => a.Name)
                .OrderBy(n => n, StringComparer.Ordinal)
                .ToList();
            var networksJson = "[" + string.Join(",", networks.Select(n => $"\"{EscapeJson(n)}\"")) + "]";
            fields.Add($"\"networks\":{networksJson}");

            // pacing
            fields.Add("\"pacing\":{\"enabled\":false,\"minIntervalMs\":0}");

            // version (Unity SDK uses local config, default to 1)
            fields.Add("\"version\":1");

            sb.Append(string.Join(",", fields));
            sb.Append("}");

            return sb.ToString();
        }

        /// <summary>
        /// Escape special characters for JSON string
        /// </summary>
        private static string EscapeJson(string input)
        {
            if (string.IsNullOrEmpty(input))
            {
                return string.Empty;
            }

            var sb = new StringBuilder();
            foreach (var c in input)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\b': sb.Append("\\b"); break;
                    case '\f': sb.Append("\\f"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < ' ')
                        {
                            sb.AppendFormat("\\u{0:X4}", (int)c);
                        }
                        else
                        {
                            sb.Append(c);
                        }
                        break;
                }
            }
            return sb.ToString();
        }

        /// <summary>
        /// Validate that local config hash matches server hash.
        /// Useful for debugging configuration sync issues.
        /// </summary>
        /// <param name="config">Local SDK configuration</param>
        /// <param name="serverHash">Hash returned from /api/v1/config/sdk/config/hash endpoint</param>
        /// <returns>true if hashes match, false otherwise</returns>
        public static bool ValidateHash(ApexConfig config, string serverHash)
        {
            if (config == null || string.IsNullOrEmpty(serverHash))
            {
                return false;
            }

            var localHash = ComputeHash(config);
            return string.Equals(localHash, serverHash, StringComparison.Ordinal);
        }
    }
}
