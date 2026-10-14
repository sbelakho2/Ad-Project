using Apex.Mediation.Adapters;
using Xunit;

public class AdapterCatalogTests
{
    [Fact]
    public void TryGet_ReturnsMetadata()
    {
        Assert.True(AdapterCatalog.TryGet("AdMob", out var metadata));
        Assert.Contains("app_id", metadata.RequiredCredentialKeys);
    }
}
