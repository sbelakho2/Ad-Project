using System.Collections.Generic;
using Apex.Mediation.Consent;
using Xunit;

public class ConsentAutoReaderTests
{
    [Fact]
    public void Read_PullsMappedKeys()
    {
        var store = new MemoryStore(new Dictionary<string, string>
        {
            {"IABTCF_TCString", "tc"},
            {"IABGPP_HDR_GppString", "gpp"},
            {"IABUSPrivacy_String", "1YNN"}
        });

        var reader = new ConsentAutoReader(store);
        var consent = reader.Read();

        Assert.Equal("tc", consent.TcfString);
        Assert.Equal("gpp", consent.GppString);
        Assert.Equal("1YNN", consent.UsPrivacyString);
    }

    private sealed class MemoryStore : IKeyValueStore
    {
        private readonly IReadOnlyDictionary<string, string> _data;
        public MemoryStore(IReadOnlyDictionary<string, string> data) => _data = data;
        public bool HasKey(string key) => _data.ContainsKey(key);
        public string GetString(string key, string defaultValue = "") => _data.TryGetValue(key, out var value) ? value : defaultValue;
    }
}
