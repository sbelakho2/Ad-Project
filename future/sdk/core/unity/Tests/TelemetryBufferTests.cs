using System;
using Apex.Mediation.Core;
using Xunit;

public class TelemetryBufferTests
{
    [Fact]
    public void Record_EnforcesCapacity()
    {
        var buffer = new TelemetryBuffer(3);
        for (var i = 0; i < 5; i++)
        {
            buffer.Record(new TelemetryTrace(i.ToString(), "adapter", "ok", TimeSpan.FromMilliseconds(10), new System.Collections.Generic.Dictionary<string, object?>()));
        }

        Assert.Equal(3, buffer.Snapshot().Count);
        Assert.Equal("2", buffer.Snapshot()[0].PlacementId);
    }
}
