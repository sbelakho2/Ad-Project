using System;
using System.Collections.Generic;
using System.Linq;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace Apex.Mediation.Editor.Console
{
    internal sealed class ConsolePlacement
    {
        [JsonProperty("id")] public string Id { get; set; } = string.Empty;
        [JsonProperty("name")] public string Name { get; set; } = string.Empty;
        [JsonProperty("type")] public string AdType { get; set; } = string.Empty;
        [JsonProperty("status")] public string Status { get; set; } = "inactive";
        [JsonProperty("app_id")] public string AppId { get; set; } = string.Empty;
        [JsonProperty("config")] public JObject? Config { get; set; }
        [JsonProperty("updated_at")] public DateTime? UpdatedAt { get; set; }
        public IReadOnlyList<string> EnabledAdapters => _enabledAdapters ??= ExtractAdapters();
        public double? FloorCpm => TryGetNumber("floorCPM") ?? TryGetNumber("floor_cpm");
        public int? TimeoutMs => (int?)TryGetNumber("timeoutMs") ?? (int?)TryGetNumber("timeout_ms");
        public double? RefreshInterval => TryGetNumber("refreshInterval") ?? TryGetNumber("refresh_interval");

        private IReadOnlyList<string>? _enabledAdapters;

        private IReadOnlyList<string> ExtractAdapters()
        {
            if (Config == null)
            {
                return Array.Empty<string>();
            }

            var candidates = new List<string>();
            var priorityNode = Config["adapterPriority"] ?? Config["priorities"] ?? Config["adapters"];
            if (priorityNode is JArray arr)
            {
                foreach (var token in arr)
                {
                    if (token.Type == JTokenType.String)
                    {
                        candidates.Add(token.Value<string>() ?? string.Empty);
                    }
                    else if (token.Type == JTokenType.Object)
                    {
                        var network = token.Value<string>("network") ?? token.Value<string>("adapter") ?? token.Value<string>("name");
                        if (!string.IsNullOrEmpty(network))
                        {
                            candidates.Add(network);
                        }
                    }
                }
            }
            else if (priorityNode is JObject obj)
            {
                candidates.AddRange(obj.Properties().Select(p => p.Name));
            }

            return candidates.Where(n => !string.IsNullOrWhiteSpace(n)).Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
        }

        private double? TryGetNumber(string key)
        {
            if (Config == null)
            {
                return null;
            }

            var token = Config.SelectToken(key);
            return token?.Value<double?>();
        }
    }

    internal sealed class ConsoleAdapterConfig
    {
        [JsonProperty("id")] public string Id { get; set; } = string.Empty;
        [JsonProperty("adapter_id")] public string AdapterId { get; set; } = string.Empty;
        [JsonProperty("status")] public string Status { get; set; } = "configured";
        [JsonProperty("created_at")] public DateTime? CreatedAt { get; set; }
        [JsonProperty("updated_at")] public DateTime? UpdatedAt { get; set; }
        [JsonProperty("config")] public JObject? Config { get; set; }
        public IReadOnlyList<string> ExposedKeys => Config == null
            ? Array.Empty<string>()
            : Config.Properties().Select(p => p.Name).ToArray();
    }

    internal sealed class ConsoleSummary
    {
        public ConsoleSummary(IReadOnlyList<ConsolePlacement> placements, IReadOnlyList<ConsoleAdapterConfig> adapters)
        {
            Placements = placements;
            Adapters = adapters;
        }

        public IReadOnlyList<ConsolePlacement> Placements { get; }
        public IReadOnlyList<ConsoleAdapterConfig> Adapters { get; }
    }

    internal sealed class ConsoleApiException : Exception
    {
        public ConsoleApiException(string message) : base(message)
        {
        }

        public ConsoleApiException(string message, Exception inner) : base(message, inner)
        {
        }
    }
}
