using System.Collections.Concurrent;

namespace Apex.Mediation.Core
{
    internal sealed class AdCache
    {
        private readonly ConcurrentDictionary<string, RenderableAd> _ads = new();

        public bool TryStore(RenderableAd ad)
        {
            _ads[ad.PlacementId] = ad;
            return true;
        }

        public void Clear()
        {
            _ads.Clear();
        }

        public bool TryTake(string placementId, out RenderableAd renderable)
        {
            if (_ads.TryGetValue(placementId, out var cached))
            {
                if (cached.IsExpired)
                {
                    _ads.TryRemove(placementId, out _);
                    renderable = null!;
                    return false;
                }

                if (cached.TryMarkShown())
                {
                    _ads.TryRemove(placementId, out _);
                    renderable = cached;
                    return true;
                }

                _ads.TryRemove(placementId, out _);
            }

            renderable = null!;
            return false;
        }
    }
}
