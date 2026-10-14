using System;
using System.Collections.Generic;
using Newtonsoft.Json.Linq;

namespace Apex.Mediation.Editor.Console
{
    internal static class ConsoleMockDataset
    {
        public static ConsoleSummary Create()
        {
            var placements = new List<ConsolePlacement>
            {
                new ConsolePlacement
                {
                    Id = "placement_interstitial_demo",
                    Name = "Demo Interstitial",
                    AdType = "interstitial",
                    Status = "active",
                    AppId = "app_demo",
                    Config = JObject.Parse("{\"adapterPriority\":[\"admob\",\"applovin\"],\"floorCPM\":18.5,\"timeoutMs\":950}")
                },
                new ConsolePlacement
                {
                    Id = "placement_rewarded_demo",
                    Name = "Rewarded Video",
                    AdType = "rewarded",
                    Status = "paused",
                    AppId = "app_demo",
                    Config = JObject.Parse("{\"adapterPriority\":[{\"network\":\"meta\"},{\"network\":\"ironsource\"}],\"floor_cpm\":25.0,\"refreshInterval\":45}")
                }
            };

            var adapters = new List<ConsoleAdapterConfig>
            {
                new ConsoleAdapterConfig
                {
                    Id = "adapter_cfg_admob",
                    AdapterId = "admob",
                    Status = "active",
                    CreatedAt = DateTime.UtcNow.AddDays(-2),
                    UpdatedAt = DateTime.UtcNow.AddHours(-3),
                    Config = JObject.Parse("{\"app_id\":\"ca-app-pub-123\",\"ad_unit_id\":\"/123/demo\"}")
                },
                new ConsoleAdapterConfig
                {
                    Id = "adapter_cfg_meta",
                    AdapterId = "facebook",
                    Status = "draft",
                    CreatedAt = DateTime.UtcNow.AddDays(-5),
                    UpdatedAt = DateTime.UtcNow.AddHours(-12),
                    Config = JObject.Parse("{\"placement_id\":\"IMG_16_9_APP_INSTALL#123\"}")
                }
            };

            return new ConsoleSummary(placements, adapters);
        }
    }
}
