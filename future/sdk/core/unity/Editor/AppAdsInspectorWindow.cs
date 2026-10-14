using System.Collections.Generic;
using System.IO;
using System.Linq;
using Apex.Mediation.Adapters;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    public sealed class AppAdsInspectorWindow : EditorWindow
    {
        private string _filePath;
        private Vector2 _scroll;
        private List<string> _fileLines = new();

        [MenuItem("Apex Mediation/app-ads.txt Inspector")]
        public static void ShowWindow()
        {
            var window = GetWindow<AppAdsInspectorWindow>("app-ads.txt Inspector");
            window.minSize = new Vector2(600, 400);
            window.LoadDefault();
        }

        private void LoadDefault()
        {
            _filePath = Path.Combine(Application.dataPath, "../app-ads.txt");
            Reload();
        }

        private void OnGUI()
        {
            EditorGUILayout.LabelField("app-ads.txt Inspector", EditorStyles.boldLabel);
            EditorGUILayout.HelpBox("Select an app-ads.txt file and compare it to the enabled adapters.", MessageType.Info);

            EditorGUILayout.BeginHorizontal();
            _filePath = EditorGUILayout.TextField("File", _filePath);
            if (GUILayout.Button("Browse", GUILayout.Width(80)))
            {
                var picked = EditorUtility.OpenFilePanel("Select app-ads.txt", Path.GetDirectoryName(_filePath) ?? Application.dataPath, "txt");
                if (!string.IsNullOrEmpty(picked))
                {
                    _filePath = picked;
                    Reload();
                }
            }
            EditorGUILayout.EndHorizontal();

            if (!File.Exists(_filePath))
            {
                EditorGUILayout.HelpBox("File not found.", MessageType.Error);
                return;
            }

            EditorGUILayout.LabelField("Status", $"Loaded {_fileLines.Count} lines");

            _scroll = EditorGUILayout.BeginScrollView(_scroll);
            foreach (var metadata in AdapterCatalog.AllAdapters())
            {
                var missing = metadata.AppAdsLines.Where(line => !_fileLines.Contains(line.ToLowerInvariant())).ToList();
                var status = missing.Any() ? "Missing" : "OK";
                var type = missing.Any() ? MessageType.Warning : MessageType.Info;
                EditorGUILayout.HelpBox($"{metadata.Name}: {status}", type);
                if (missing.Any())
                {
                    EditorGUILayout.LabelField("Add:", string.Join("\n", metadata.AppAdsLines));
                }
            }
            EditorGUILayout.EndScrollView();
        }

        private void Reload()
        {
            if (File.Exists(_filePath))
            {
                _fileLines = File.ReadAllLines(_filePath)
                    .Select(l => l.Trim().ToLowerInvariant())
                    .Where(l => !string.IsNullOrEmpty(l))
                    .ToList();
            }
        }
    }
}
