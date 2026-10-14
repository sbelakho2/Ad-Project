using System.Collections.Generic;
using Apex.Mediation.Core;
using Xunit;

public class SdkModeTests
{
    [Fact]
    public void ShouldUseS2S_WhenModeByoAndMissingCreds_ReturnsFalse()
    {
        var config = new ApexConfig();
        var adapter = new NetworkAdapterDescriptor("TestNet", supportsS2S: true, requiredCredentialKeys: new[] { "api_key" });
        config.EnableAdapter(adapter);
        var store = new CredentialStore(NullAdapterConfigProvider.Instance);

        Assert.False(SdkModeDecider.ShouldUseS2S(SdkMode.BYO, config.EnabledAdapters, store));
    }

    [Fact]
    public void ShouldUseS2S_WhenHybrid_ReturnsTrue()
    {
        var config = new ApexConfig { Mode = SdkMode.HYBRID };
        var store = new CredentialStore(NullAdapterConfigProvider.Instance);

        Assert.True(SdkModeDecider.ShouldUseS2S(config.Mode, config.EnabledAdapters, store));
    }

    [Fact]
    public void CredentialStore_HasS2SCreds_ValidatesKeys()
    {
        var provider = new FakeAdapterProvider(new Dictionary<string, object?>
        {
            {"api_key", "123"}
        });
        var store = new CredentialStore(provider);
        var adapter = new NetworkAdapterDescriptor("TestNet", true, new[] { "api_key" });

        Assert.True(store.HasS2SCreds(new[] { adapter }));
    }

    private sealed class FakeAdapterProvider : IAdapterConfigProvider
    {
        private readonly IReadOnlyDictionary<string, object?> _config;
        public FakeAdapterProvider(IReadOnlyDictionary<string, object?> config) => _config = config;
        public IReadOnlyDictionary<string, object?>? GetConfig(string network) => _config;
    }
}
