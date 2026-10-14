using System.Collections.Generic;
using System.Linq;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    internal sealed class AdapterCredentialSettings : ScriptableObject
    {
        private const string AssetPath = "Assets/Editor/ApexAdapterCredentials.asset";

        [SerializeField]
        private List<NetworkCredentialEntry> entries = new();

        public IReadOnlyList<NetworkCredentialEntry> Entries => entries;

        public static AdapterCredentialSettings LoadOrCreate()
        {
            var settings = AssetDatabase.LoadAssetAtPath<AdapterCredentialSettings>(AssetPath);
            if (settings == null)
            {
                settings = CreateInstance<AdapterCredentialSettings>();
                AssetDatabase.CreateAsset(settings, AssetPath);
                AssetDatabase.SaveAssets();
            }

            return settings;
        }

        public void Upsert(string network, IReadOnlyDictionary<string, string> fields)
        {
            var existing = entries.Find(e => e.Network == network);
            if (existing == null)
            {
                existing = new NetworkCredentialEntry { Network = network };
                entries.Add(existing);
            }

            existing.UpdateFields(fields);

            EditorUtility.SetDirty(this);
        }

        public bool TryGet(string network, out NetworkCredentialEntry entry)
        {
            entry = entries.FirstOrDefault(e => e.Network == network);
            return entry != null;
        }

        [System.Serializable]
        public sealed class NetworkCredentialEntry
        {
            public string Network;
            public string Key;
            [TextArea]
            public string Secret;
            [SerializeField]
            private List<CredentialField> fields = new();

            public IReadOnlyDictionary<string, string> AsDictionary()
            {
                if (fields == null || fields.Count == 0)
                {
                    var legacy = new Dictionary<string, string>();
                    if (!string.IsNullOrEmpty(Key)) legacy["key"] = Key;
                    if (!string.IsNullOrEmpty(Secret)) legacy["secret"] = Secret;
                    return legacy;
                }

                return fields.ToDictionary(f => f.name, f => f.value);
            }

            public void UpdateFields(IReadOnlyDictionary<string, string> newFields)
            {
                if (newFields == null || newFields.Count == 0)
                {
                    return;
                }

                fields ??= new List<CredentialField>();
                foreach (var kv in newFields)
                {
                    var existing = fields.FirstOrDefault(f => f.name == kv.Key);
                    if (existing == null)
                    {
                        fields.Add(new CredentialField { name = kv.Key, value = kv.Value });
                    }
                    else
                    {
                        existing.value = kv.Value;
                    }
                }

                if (newFields.TryGetValue("key", out var keyValue))
                {
                    Key = keyValue;
                }

                if (newFields.TryGetValue("secret", out var secretValue))
                {
                    Secret = secretValue;
                }
            }
        }

        [System.Serializable]
        public sealed class CredentialField
        {
            public string name;
            public string value;
        }
    }
}
