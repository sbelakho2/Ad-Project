using System.Collections.Generic;
using Apex.Mediation.Internal;
using Xunit;

public class RedactorTests
{
    [Fact]
    public void Redact_ReplacesSensitiveKeys()
    {
        var raw = "api_key=abcd";
        var redacted = Redactor.Redact(raw);
        Assert.Equal("****=abcd", redacted);
    }

    [Fact]
    public void RedactMap_MasksValues()
    {
        var map = new Dictionary<string, object?> { {"account_id", "foo"}, {"nonSecret", "bar"} };
        var redacted = Redactor.RedactMap(map);
        Assert.Equal("****", redacted["account_id"]);
        Assert.Equal("bar", redacted["nonSecret"]);
    }
}
