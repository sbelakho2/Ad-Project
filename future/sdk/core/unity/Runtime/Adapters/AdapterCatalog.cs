using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;

namespace Apex.Mediation.Adapters
{
    public static class AdapterCatalog
    {
        private static readonly IReadOnlyDictionary<string, AdapterMetadata> Catalog;

        static AdapterCatalog()
        {
            var entries = new[]
            {
                new AdapterMetadata(
                    "AdMob",
                    new[] { "app_id", "ad_unit_id" },
                    new[] { "google.com, pub-xxxxxxxxxxxxxxxx, DIRECT, f08c47fec0942fa0" },
                    "https://developers.google.com/admob/unity/quick-start"),
                new AdapterMetadata(
                    "AppLovin",
                    new[] { "sdk_key", "ad_unit_id" },
                    new[] { "applovin.com, 1234567890abcdef1234567890abcdef, DIRECT" },
                    "https://dash.applovin.com/documentation/mediation/unity/getting-started"),
                new AdapterMetadata(
                    "ironSource",
                    new[] { "app_key", "placement_name" },
                    new[] { "ironsrc.com, 123456789, DIRECT" },
                    "https://developers.is.com/ironsource-mobile/unity/unity-plugin-installation/"),
                new AdapterMetadata(
                    "UnityAds",
                    new[] { "game_id", "placement_id" },
                    new[] { "unityads.unity3d.com, 1234567, DIRECT" },
                    "https://unity.com/solutions/gaming/operate/monetization"),
                new AdapterMetadata(
                    "Meta Audience Network",
                    new[] { "app_id", "placement_id" },
                    new[] { "facebook.com, 1234567890, DIRECT, c3e20eee3f780d68" },
                    "https://developers.facebook.com/docs/audience-network/"),
                new AdapterMetadata(
                    "Vungle",
                    new[] { "app_id" },
                    new[] { "vungle.com, 1234567890, DIRECT" },
                    "https://support.vungle.com/hc/en-us/categories/115000468351"),
                new AdapterMetadata(
                    "Chartboost",
                    new[] { "app_id", "app_signature" },
                    new[] { "chartboost.com, 1234567890, DIRECT" },
                    "https://answers.chartboost.com/en-us/child_article/unity-getting-started"),
                new AdapterMetadata(
                    "Pangle",
                    new[] { "app_id", "slot_id" },
                    new[] { "bytedance.com, 1234567890, DIRECT" },
                    "https://www.pangleglobal.com/unity"),
                new AdapterMetadata(
                    "Mintegral",
                    new[] { "app_id", "app_key" },
                    new[] { "mintegral.com, 1234567890, DIRECT" },
                    "https://support.mintegral.com/docs/unity"),
                new AdapterMetadata(
                    "AdColony",
                    new[] { "app_id", "zone_id" },
                    new[] { "adcolony.com, 1234567890, DIRECT" },
                    "https://github.com/AdColony/AdColony-Unity-SDK-3"),
                new AdapterMetadata(
                    "Tapjoy",
                    new[] { "sdk_key", "placement_name" },
                    new[] { "tapjoy.com, 1234567890, DIRECT" },
                    "https://dev.tapjoy.com/unity-sdk/set-up-unity/"),
                new AdapterMetadata(
                    "Moloco",
                    new[] { "publisher_id", "api_key" },
                    new[] { "moloco.com, 1234567890, DIRECT" },
                    "https://www.moloco.com/resources"),
                new AdapterMetadata(
                    "Fyber",
                    new[] { "app_id" },
                    new[] { "fyber.com, 1234567890, DIRECT" },
                    "https://developer.digitalturbine.com/hc/en-us/categories/360004645493-Unity"),
                new AdapterMetadata(
                    "Smaato",
                    new[] { "publisher_id", "ad_space_id" },
                    new[] { "smaato.com, 1234567890, DIRECT" },
                    "https://developers.smaato.com/publishers/integration-guides/unity"),
                new AdapterMetadata(
                    "Amazon Publisher Services",
                    new[] { "account_id", "slot_uuid" },
                    new[] { "amazon-adsystem.com, 1234567890, DIRECT" },
                    "https://aps.amazon.com/aps/aps-mediation"),
                new AdapterMetadata(
                    "MockAdapter",
                    new[] { "api_key" },
                    new[] { "mocknetwork.example, 000000, DIRECT" },
                    "https://docs.example.com/mock")
            };

            Catalog = new ReadOnlyDictionary<string, AdapterMetadata>(entries.ToDictionary(e => e.Name));
        }

        public static bool TryGet(string network, out AdapterMetadata metadata)
        {
            if (string.IsNullOrWhiteSpace(network))
            {
                metadata = null!;
                return false;
            }

            return Catalog.TryGetValue(network, out metadata!);
        }

        public static IEnumerable<AdapterMetadata> AllAdapters() => Catalog.Values;
    }
}
