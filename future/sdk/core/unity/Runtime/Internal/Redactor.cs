using System.Collections.Generic;
using System.Text.RegularExpressions;

namespace Apex.Mediation.Internal
{
    internal static class Redactor
    {
        private static readonly Regex SensitivePattern = new(
            "\\b(" + string.Join("|", new[]
            {
                "api_key",
                "access_key",
                "account_id",
                "token",
                "api_token",
                "placement_id",
                "app_token",
                "secret",
                "password"
            }) + ")\\b",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        public static string Redact(string input)
        {
            if (string.IsNullOrEmpty(input))
            {
                return input ?? string.Empty;
            }

            return SensitivePattern.Replace(input, "****");
        }

        public static IReadOnlyDictionary<string, object?> RedactMap(IReadOnlyDictionary<string, object?> map)
        {
            var copy = new Dictionary<string, object?>(map.Count);
            foreach (var kv in map)
            {
                if (SensitivePattern.IsMatch(kv.Key))
                {
                    copy[kv.Key] = "****";
                    continue;
                }

                copy[kv.Key] = kv.Value;
            }

            return copy;
        }
    }
}
