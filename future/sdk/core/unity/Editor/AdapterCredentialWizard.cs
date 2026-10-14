using System.Collections.Generic;
using System.Linq;
using Apex.Mediation.Adapters;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    internal sealed class AdapterCredentialWizard : EditorWindow
    {
        private AdapterMetadata[] _adapters = AdapterCatalog.AllAdapters().ToArray();
        private int _selectedIndex;
        private Dictionary<string, string> _fields = new();
        private string _status = string.Empty;

        [MenuItem("Apex Mediation/Credential Wizard")]
        public static void ShowWindow()
        {
            var window = GetWindow<AdapterCredentialWizard>(true, "Credential Wizard");
            window.minSize = new Vector2(420, 320);
            window.Reload();
        }

        private void Reload()
        {
            LoadFields(CurrentAdapter.Name);
        }

        private AdapterMetadata CurrentAdapter => _adapters[Mathf.Clamp(_selectedIndex, 0, _adapters.Length - 1)];

        private void LoadFields(string network)
        {
            var settings = AdapterCredentialSettings.LoadOrCreate();
            if (settings.TryGet(network, out var entry))
            {
                _fields = entry.AsDictionary().ToDictionary(kvp => kvp.Key, kvp => kvp.Value);
            }
            else
            {
                _fields = new Dictionary<string, string>();
            }
        }

        private void OnGUI()
        {
            GUILayout.Label("Network Credentials", EditorStyles.boldLabel);
            var names = _adapters.Select(a => a.Name).ToArray();
            var newIndex = EditorGUILayout.Popup("Adapter", _selectedIndex, names);
            if (newIndex != _selectedIndex)
            {
                _selectedIndex = newIndex;
                LoadFields(CurrentAdapter.Name);
            }

            EditorGUILayout.LabelField("Documentation", CurrentAdapter.HelpUrl);

            foreach (var key in CurrentAdapter.RequiredCredentialKeys)
            {
                _fields[key] = EditorGUILayout.TextField(ObjectNames.NicifyVariableName(key), _fields.TryGetValue(key, out var existing) ? existing : string.Empty);
            }

            EditorGUILayout.HelpBox("Values are encrypted locally and never uploaded.", MessageType.Info);

            using (new EditorGUILayout.HorizontalScope())
            {
                if (GUILayout.Button("Save"))
                {
                    Save();
                }

                if (GUILayout.Button("Validate"))
                {
                    Save();
                    var result = AdapterValidationUtility.Validate(CurrentAdapter.Name);
                    _status = result.Success ? "Validation passed" : result.Message;
                }
            }

            if (!string.IsNullOrEmpty(_status))
            {
                EditorGUILayout.HelpBox(_status, MessageType.None);
            }
        }

        private void Save()
        {
            var sanitized = _fields.Where(kvp => !string.IsNullOrEmpty(kvp.Value))
                .ToDictionary(kvp => kvp.Key, kvp => kvp.Value);
            var settings = AdapterCredentialSettings.LoadOrCreate();
            settings.Upsert(CurrentAdapter.Name, sanitized);
            _status = "Saved";
        }
    }
}
