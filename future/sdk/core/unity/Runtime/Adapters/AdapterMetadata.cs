using System.Collections.Generic;

namespace Apex.Mediation.Adapters
{
    public sealed class AdapterMetadata
    {
        public AdapterMetadata(string name, IReadOnlyList<string> requiredCredentialKeys, IReadOnlyList<string> appAdsLines, string helpUrl)
        {
            Name = name;
            RequiredCredentialKeys = requiredCredentialKeys;
            AppAdsLines = appAdsLines;
            HelpUrl = helpUrl;
        }

        public string Name { get; }
        public IReadOnlyList<string> RequiredCredentialKeys { get; }
        public IReadOnlyList<string> AppAdsLines { get; }
        public string HelpUrl { get; }
    }
}
