using System;
using System.Collections.Generic;
using System.Linq;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    public sealed class MigrationStudioWindow : EditorWindow
    {
        private string _inputJson = string.Empty;
        private string _status = "Paste MAX/ironSource/Unity Ads export JSON to begin.";
        private MigrationResult? _result;
        private Vector2 _scroll;

        [MenuItem("Apex Mediation/Migration Studio")]
        public static void ShowWindow()
        {
            var window = GetWindow<MigrationStudioWindow>("Migration Studio");
            window.minSize = new Vector2(640, 480);
        }

        private void OnGUI()
        {
            EditorGUILayout.LabelField("Migration Studio", EditorStyles.boldLabel);
            EditorGUILayout.HelpBox("Import existing mediation configs and preview the Apex config export.", MessageType.Info);

            using (new EditorGUILayout.HorizontalScope())
            {
                if (GUILayout.Button("Paste from clipboard"))
                {
                    _inputJson = EditorGUIUtility.systemCopyBuffer;
                }

                if (GUILayout.Button("Clear"))
                {
                    _inputJson = string.Empty;
                    _result = null;
                    _status = "";
                }

                if (GUILayout.Button("Parse"))
                {
                    _result = MigrationImporter.TryImport(_inputJson, out _status);
                }
            }

            EditorGUILayout.LabelField("Input JSON");
            _inputJson = EditorGUILayout.TextArea(_inputJson, GUILayout.Height(160));

            if (!string.IsNullOrEmpty(_status))
            {
                EditorGUILayout.HelpBox(_status, MessageType.Info);
            }

            if (_result == null)
            {
                return;
            }

            EditorGUILayout.LabelField($"Detected source: {_result.Source}");
            _scroll = EditorGUILayout.BeginScrollView(_scroll);
            foreach (var placement in _result.Placements)
            {
                EditorGUILayout.LabelField($"{placement.PlacementId} · {placement.Format} · {placement.Network}");
            }
            EditorGUILayout.EndScrollView();

            if (GUILayout.Button("Copy Apex config JSON"))
            {
                EditorGUIUtility.systemCopyBuffer = _result.ToApexConfigJson();
                _status = "Copied Apex config to clipboard";
            }
        }
    }

    internal sealed class MigrationResult
    {
        public string Source { get; }
        public IReadOnlyList<MigrationPlacement> Placements { get; }

        public MigrationResult(string source, IReadOnlyList<MigrationPlacement> placements)
        {
            Source = source;
            Placements = placements;
        }

        public string ToApexConfigJson()
        {
            var dto = new ApexConfigExport
            {
                mode = "BYO",
                placements = Placements.Select(p => new ApexPlacement
                {
                    placementId = p.PlacementId,
                    format = p.Format,
                    network = p.Network
                }).ToArray()
            };
            return JsonUtility.ToJson(dto, true);
        }
    }

    internal sealed class MigrationPlacement
    {
        public string PlacementId { get; }
        public string Format { get; }
        public string Network { get; }

        public MigrationPlacement(string placementId, string format, string network)
        {
            PlacementId = placementId;
            Format = format;
            Network = network;
        }
    }

    internal static class MigrationImporter
    {
        public static MigrationResult? TryImport(string json, out string status)
        {
            if (string.IsNullOrWhiteSpace(json))
            {
                status = "Input is empty";
                return null;
            }

            if (TryParse<UnityAdsExport>(json, e => e.placements, "Unity Ads", out var result, out status))
            {
                return result;
            }

            if (TryParse<MaxExport>(json, e => e.adUnits, "AppLovin MAX", out result, out status))
            {
                return result;
            }

            if (TryParse<IronSourceExport>(json, e => e.placements, "ironSource", out result, out status))
            {
                return result;
            }

            status = "Unsupported format";
            return null;
        }

        private static bool TryParse<T>(string json, Func<T, IEnumerable<PlacementStub>> selector, string source, out MigrationResult? result, out string status)
            where T : class, new()
        {
            try
            {
                var data = JsonUtility.FromJson<T>(json);
                var placements = selector(data);
                if (placements == null)
                {
                    result = null;
                    status = "No placements found";
                    return false;
                }

                var list = placements
                    .Select(p => new MigrationPlacement(
                        p.ResolvePlacementId(),
                        p.ResolveFormat(),
                        p.ResolveNetwork(source)))
                    .Where(p => !string.IsNullOrEmpty(p.PlacementId))
                    .ToList();

                if (!list.Any())
                {
                    result = null;
                    status = "No valid placements";
                    return false;
                }

                result = new MigrationResult(source, list);
                status = $"Imported {list.Count} placements";
                return true;
            }
            catch (Exception ex)
            {
                result = null;
                status = ex.Message;
                return false;
            }
        }
    }

    [Serializable]
    internal sealed class PlacementStub
    {
        public string placementId;
        public string adUnitId;
        public string id;
        public string format;
        public string adFormat;
        public string network;
        public string source;
        public string mediationNetwork;

        public string ResolvePlacementId()
        {
            return placementId ?? adUnitId ?? id ?? string.Empty;
        }

        public string ResolveFormat()
        {
            return format ?? adFormat ?? "interstitial";
        }

        public string ResolveNetwork(string fallback)
        {
            return network ?? mediationNetwork ?? source ?? fallback;
        }
    }

    [Serializable]
    internal sealed class UnityAdsExport
    {
        public PlacementStub[] placements;
    }

    [Serializable]
    internal sealed class MaxExport
    {
        public PlacementStub[] adUnits;
    }

    [Serializable]
    internal sealed class IronSourceExport
    {
        public PlacementStub[] placements;
    }

    [Serializable]
    internal sealed class ApexConfigExport
    {
        public string mode;
        public ApexPlacement[] placements;
    }

    [Serializable]
    internal sealed class ApexPlacement
    {
        public string placementId;
        public string format;
        public string network;
    }
}
