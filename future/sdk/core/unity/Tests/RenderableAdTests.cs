using System;
using Apex.Mediation.Core;
using Xunit;

public class RenderableAdTests
{
    [Fact]
    public void TryMarkShown_AllowsSingleUse()
    {
        var ad = new RenderableAd("placement", "adapter", TimeSpan.FromMinutes(5), new MutableClock());
        Assert.True(ad.TryMarkShown());
        Assert.False(ad.TryMarkShown());
    }

    [Fact]
    public void TryTake_FromCacheHonorsTtl()
    {
        var cache = new AdCache();
        var clock = new MutableClock();
        var ad = new RenderableAd("placement", "adapter", TimeSpan.FromMilliseconds(10), clock);
        cache.TryStore(ad);

        Assert.True(cache.TryTake("placement", out _));
        Assert.False(cache.TryTake("placement", out _));
    }

    [Fact]
    public void TryTake_FailsAfterExpiry()
    {
        var cache = new AdCache();
        var clock = new MutableClock();
        var ad = new RenderableAd("placement", "adapter", TimeSpan.FromMilliseconds(5), clock);
        cache.TryStore(ad);

        clock.Advance(TimeSpan.FromMilliseconds(20));

        Assert.False(cache.TryTake("placement", out _));
    }

    [Fact]
    public void TryStore_ReplacesExistingPlacementEntry()
    {
        var cache = new AdCache();
        var clock = new MutableClock();
        var first = new RenderableAd("placement", "adapterA", TimeSpan.FromMinutes(1), clock);
        var second = new RenderableAd("placement", "adapterB", TimeSpan.FromMinutes(1), clock);

        cache.TryStore(first);
        cache.TryStore(second);

        Assert.True(cache.TryTake("placement", out var final));
        Assert.Equal("adapterB", final.Adapter);
    }

    [Fact]
    public void TryTake_SurvivesBackwardDrift()
    {
        var clock = new MutableClock();
        var cache = new AdCache();
        cache.TryStore(new RenderableAd("placement", "adapter", TimeSpan.FromMinutes(5), clock));

        clock.Advance(TimeSpan.FromMinutes(-10));

        Assert.True(cache.TryTake("placement", out var renderable));
        Assert.Equal("adapter", renderable.Adapter);
    }

    [Fact]
    public void TryTake_ExpiresAfterForwardDriftBeyondTtl()
    {
        var clock = new MutableClock();
        var cache = new AdCache();
        cache.TryStore(new RenderableAd("placement", "adapter", TimeSpan.FromMinutes(5), clock));

        clock.Advance(TimeSpan.FromMinutes(11));

        Assert.False(cache.TryTake("placement", out _));
    }
}

internal sealed class MutableClock : IClock
{
    public TimeSpan Now { get; private set; } = TimeSpan.Zero;

    public void Advance(TimeSpan delta)
    {
        Now += delta;
    }
}
