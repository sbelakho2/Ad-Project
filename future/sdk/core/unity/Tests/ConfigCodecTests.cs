using Apex.Mediation.Core;
using Xunit;

public class ConfigCodecTests
{
    [Fact]
    public void ExportAndImport_RoundTrips()
    {
        var config = new ApexConfig { AppId = "app" };
        config.EnableAdapter(new NetworkAdapterDescriptor("MockAdapter", false, new[] { "api_key" }));
        config.DefinePlacement("placement", PlacementFormat.Interstitial, 1.5);

        var json = ConfigCodec.Export(config, "secret");
        var imported = ConfigCodec.Import(json, "secret");

        Assert.Equal(config.AppId, imported.AppId);
        Assert.Single(imported.Placements);
        Assert.Single(imported.EnabledAdapters);
    }

    [Fact]
    public void Import_InvalidSignature_Throws()
    {
        var bad = "{\"appId\":\"a\",\"signature\":\"x\"}";
        Assert.Throws<System.InvalidOperationException>(() => ConfigCodec.Import(bad, "secret"));
    }
}
