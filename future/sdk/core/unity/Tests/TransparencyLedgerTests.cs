using System;
using System.Collections.Generic;
using Apex.Mediation.Core;
using Xunit;

public class TransparencyLedgerTests
{
    [Fact]
    public void Record_BuildsChain()
    {
        var ledger = new TransparencyLedger();
        var trace = new TelemetryTrace("p", "adapter", "ok", TimeSpan.FromMilliseconds(10), new Dictionary<string, object?>());
        ledger.Record(trace);
        var proof = ledger.Snapshot();
        Assert.Single(proof);
        Assert.False(string.IsNullOrEmpty(proof[0].Hash));
    }
}
