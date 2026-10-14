using System.Collections.Generic;

namespace Apex.Mediation.Consent
{
    internal interface IKeyValueStore
    {
        bool HasKey(string key);
        string GetString(string key, string defaultValue = "");
    }

    internal sealed class ConsentAutoReader
    {
        private readonly IKeyValueStore _store;
        private readonly Dictionary<string, string> _keyMap = new()
        {
            {"IABTCF_TCString", "tcf"},
            {"IABGPP_HDR_GppString", "gpp"},
            {"IABUSPrivacy_String", "usp"}
        };

        public ConsentAutoReader(IKeyValueStore store)
        {
            _store = store;
        }

        public ConsentOptions Read()
        {
            var options = new ConsentOptions();
            foreach (var kvp in _keyMap)
            {
                if (_store.HasKey(kvp.Key))
                {
                    var value = _store.GetString(kvp.Key, string.Empty);
                    if (string.IsNullOrEmpty(value))
                    {
                        continue;
                    }

                    switch (kvp.Value)
                    {
                        case "tcf":
                            options.TcfString = value;
                            break;
                        case "gpp":
                            options.GppString = value;
                            break;
                        case "usp":
                            options.UsPrivacyString = value;
                            break;
                    }
                }
            }

            return options;
        }
    }
}
