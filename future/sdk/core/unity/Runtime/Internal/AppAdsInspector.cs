using System.Collections.Generic;
using System.IO;
using System.Linq;
using Apex.Mediation.Adapters;
using Apex.Mediation.Core;
#if UNITY_2020_3_OR_NEWER
using UnityEngine;
#endif

namespace Apex.Mediation.Internal
{
    internal static class AppAdsInspector
    {
        public static void WarnIfMissing(IEnumerable<NetworkAdapterDescriptor> adapters)
        {
#if UNITY_EDITOR || UNITY_STANDALONE
            var path = ResolveAppAdsPath();
            if (string.IsNullOrEmpty(path) || !File.Exists(path))
            {
                Logger.LogDebug("app-ads.txt not found — skipping inspector");
                return;
            }

            var present = new HashSet<string>(File.ReadAllLines(path).Select(l => l.Trim()), System.StringComparer.OrdinalIgnoreCase);
            foreach (var adapter in adapters)
            {
                if (AdapterCatalog.TryGet(adapter.Name, out var metadata))
                {
                    foreach (var required in metadata.AppAdsLines)
                    {
                        if (!present.Contains(required))
                        {
                            Logger.LogWarning($"app-ads.txt missing line for {adapter.Name}: {required}");
                        }
                    }
                }
            }
#endif
        }

        private static string ResolveAppAdsPath()
        {
#if UNITY_2020_3_OR_NEWER
            var streaming = Path.Combine(Application.streamingAssetsPath, "app-ads.txt");
            if (File.Exists(streaming))
            {
                return streaming;
            }
            return Path.Combine(Application.dataPath, "../app-ads.txt");
#else
            return string.Empty;
#endif
        }
    }
}
