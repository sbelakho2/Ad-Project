#if UNITY_2020_3_OR_NEWER
using UnityEngine;
#endif

namespace Apex.Mediation.Consent
{
    internal sealed class PlayerPrefsStore : IKeyValueStore
    {
        public bool HasKey(string key)
        {
#if UNITY_2020_3_OR_NEWER
            return PlayerPrefs.HasKey(key);
#else
            return false;
#endif
        }

        public string GetString(string key, string defaultValue = "")
        {
#if UNITY_2020_3_OR_NEWER
            return PlayerPrefs.GetString(key, defaultValue);
#else
            return defaultValue;
#endif
        }
    }
}
