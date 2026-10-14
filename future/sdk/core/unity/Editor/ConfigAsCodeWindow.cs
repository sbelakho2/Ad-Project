using Apex.Mediation.Core;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    public sealed class ConfigAsCodeWindow : EditorWindow
    {
        private string _signingKey = "local-dev-key";
        private string _exportJson = "";
        private string _importJson = "";
        private string _status = "Paste a config JSON or export the active ScriptableObject.";

        [MenuItem("Apex Mediation/Config-as-Code")]
        public static void ShowWindow()
        {
            var window = GetWindow<ConfigAsCodeWindow>("Config as Code");
            window.minSize = new Vector2(640, 480);
        }

        private void OnGUI()
        {
            EditorGUILayout.LabelField("Config-as-Code", EditorStyles.boldLabel);
            _signingKey = EditorGUILayout.TextField("Signing Key", _signingKey);

            EditorGUILayout.Space();
            DrawExport();
            EditorGUILayout.Space();
            DrawImport();

            if (!string.IsNullOrEmpty(_status))
            {
                EditorGUILayout.HelpBox(_status, MessageType.Info);
            }
        }

        private void DrawExport()
        {
            EditorGUILayout.LabelField("Export", EditorStyles.boldLabel);
            if (GUILayout.Button("Export Mock Config"))
            {
                var config = BuildMockConfig();
                _exportJson = ConfigCodec.Export(config, _signingKey);
                EditorGUIUtility.systemCopyBuffer = _exportJson;
                _status = "Exported mock config to clipboard";
            }

            _exportJson = EditorGUILayout.TextArea(_exportJson, GUILayout.Height(150));
        }

        private void DrawImport()
        {
            EditorGUILayout.LabelField("Import", EditorStyles.boldLabel);
            _importJson = EditorGUILayout.TextArea(_importJson, GUILayout.Height(150));
            if (GUILayout.Button("Validate & Load"))
            {
                try
                {
                    var config = ConfigCodec.Import(_importJson, _signingKey);
                    _status = $"Config valid: {config.Placements.Count} placements, {config.EnabledAdapters.Count} adapters";
                }
                catch (System.Exception ex)
                {
                    _status = ex.Message;
                }
            }
        }

        private static ApexConfig BuildMockConfig()
        {
            var config = new ApexConfig { AppId = "demo-app" };
            config.EnableAdapter(new NetworkAdapterDescriptor("MockAdapter", false, new[] { "api_key" }));
            config.DefinePlacement("demo-interstitial", PlacementFormat.Interstitial, 2.5);
            return config;
        }
    }
}
