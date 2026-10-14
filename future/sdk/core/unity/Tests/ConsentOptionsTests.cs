using Apex.Mediation.Consent;
using Xunit;

public class ConsentOptionsTests
{
    [Fact]
    public void AsAdapterMap_NormalizesCanonicalKeys()
    {
        var options = new ConsentOptions
        {
            GdprApplies = true,
            TcfString = "tc-string",
            GppString = "gpp",
            UsPrivacyString = "1YNN",
            CoppaApplies = true,
            AttStatus = "authorized",
            LimitAdTracking = true
        };

        var map = options.AsAdapterMap();

        Assert.Equal("1", map["gdpr_applies"]);
        Assert.Equal("tc-string", map["iab_tcf_v2"]);
        Assert.Equal("gpp", map["iab_gpp"]);
        Assert.Equal("1YNN", map["iab_us_privacy"]);
        Assert.Equal("1YNN", map["us_privacy"]);
        Assert.Equal("1", map["coppa"]);
        Assert.Equal("authorized", map["att_status"]);
        Assert.True(map.TryGetValue("limit_ad_tracking", out var lat) && lat is bool b && b);
    }

    [Fact]
    public void AsAdapterMap_OmitsEmptyValues()
    {
        var options = new ConsentOptions();
        var map = options.AsAdapterMap();
        Assert.Empty(map);
    }
}
