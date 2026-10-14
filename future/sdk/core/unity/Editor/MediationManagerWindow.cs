using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using Apex.Mediation;
using Apex.Mediation.Adapters;
using Apex.Mediation.Core;
using Apex.Mediation.Editor.Console;
using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor
{
    public sealed class MediationManagerWindow : EditorWindow
    {
        private readonly List<ConsolePlacement> _placements = new();
        private readonly List<ConsoleAdapterConfig> _adapterConfigs = new();
        private Vector2 _placementScroll;
        private Vector2 _adapterScroll;
        private string _search = string.Empty;
        private string _status = string.Empty;
        private bool _isLoading;
        private string? _loadError;
        private DateTime? _lastSync;
        private ConsoleApiClient? _apiClient;
        private CancellationTokenSource? _refreshCts;

        [MenuItem("Apex Mediation/Manager")]
        public static void ShowWindow()
        {
            var window = GetWindow<MediationManagerWindow>("Apex Mediation Manager");
            window.minSize = new Vector2(720, 480);
        }

        private void OnEnable()
        {
            _apiClient ??= new ConsoleApiClient();
            RefreshConsoleData();
        }

        private void OnDisable()
        {
            _refreshCts?.Cancel();
            _refreshCts?.Dispose();
            _refreshCts = null;
            _apiClient?.Dispose();
            _apiClient = null;
        }

        private void OnGUI()
        {
            EditorGUILayout.LabelField("Apex Mediation", EditorStyles.boldLabel);
            EditorGUILayout.HelpBox("Console-aware tooling for BYO mediation. Manage placements, adapter credentials, and live diagnostics from one window.", MessageType.Info);

            DrawConsoleStatus();
            EditorGUILayout.Space();
            DrawQuickActions();
            EditorGUILayout.Space();
            _search = EditorGUILayout.TextField("Search", _search);
            DrawPlacements();
            EditorGUILayout.Space();
            DrawAdapters();
            EditorGUILayout.Space();
            DrawDiagnostics();

            if (!string.IsNullOrEmpty(_status))
            {
                EditorGUILayout.HelpBox(_status, MessageType.None);
            }
        }

        private void DrawQuickActions()
        {
            EditorGUILayout.LabelField("Quick Actions", EditorStyles.boldLabel);
            using (new EditorGUILayout.HorizontalScope())
            {
                if (GUILayout.Button("Credential Wizard"))
                {
                    AdapterCredentialWizard.ShowWindow();
                }

                if (GUILayout.Button("app-ads.txt Inspector"))
                {
                    AppAdsInspectorWindow.ShowWindow();
                }

                if (GUILayout.Button("Migration Studio"))
                {
                    MigrationStudioWindow.ShowWindow();
                }

                if (GUILayout.Button("Docs"))
                {
                    Application.OpenURL("https://apexmediation.ee/docs");
                }

                if (GUILayout.Button("Config-as-Code"))
                {
                    ConfigAsCodeWindow.ShowWindow();
                }
            }
        }

        private void DrawAdapters()
        {
            EditorGUILayout.LabelField("Adapter Configs", EditorStyles.boldLabel);
            var consoleAdapters = FilterConsoleAdapters(_search);

            if (_isLoading && consoleAdapters.Count == 0)
            {
                EditorGUILayout.HelpBox("Syncing adapter configs…", MessageType.Info);
            }

            if (!_isLoading && consoleAdapters.Count == 0)
            {
                EditorGUILayout.HelpBox("No adapter configs found in the console for this publisher.", MessageType.Warning);
            }

            _adapterScroll = EditorGUILayout.BeginScrollView(_adapterScroll, GUILayout.Height(220));
            foreach (var adapter in consoleAdapters)
            {
                EditorGUILayout.BeginVertical(EditorStyles.helpBox);
                EditorGUILayout.LabelField($"{adapter.AdapterId} ({adapter.Status})", EditorStyles.largeLabel);
                EditorGUILayout.LabelField("Keys", adapter.ExposedKeys.Count == 0 ? "—" : string.Join(", ", adapter.ExposedKeys));
                if (adapter.UpdatedAt.HasValue)
                {
                    EditorGUILayout.LabelField("Last Updated", adapter.UpdatedAt.Value.ToLocalTime().ToString("g"));
                }

                using (new EditorGUILayout.HorizontalScope())
                {
                    if (GUILayout.Button("Validate"))
                    {
                        RunValidation(adapter.AdapterId);
                    }

                    if (GUILayout.Button("Copy Keys"))
                    {
                        EditorGUIUtility.systemCopyBuffer = string.Join(",", adapter.ExposedKeys);
                        _status = $"Copied keys for {adapter.AdapterId}";
                    }
                }
                EditorGUILayout.EndVertical();
            }
            EditorGUILayout.EndScrollView();

            EditorGUILayout.Space();
            EditorGUILayout.LabelField("Adapter Catalog Reference", EditorStyles.boldLabel);
            using (new EditorGUILayout.VerticalScope(EditorStyles.helpBox))
            {
                EditorGUILayout.HelpBox("Reference data sourced from the SDK package to help configure app-ads.txt and required credentials.", MessageType.None);
                foreach (var metadata in FilterCatalogAdapters(_search))
                {
                    EditorGUILayout.BeginVertical(EditorStyles.helpBox);
                    EditorGUILayout.LabelField(metadata.Name, EditorStyles.boldLabel);
                    EditorGUILayout.LabelField("Required Keys", string.Join(", ", metadata.RequiredCredentialKeys));
                    EditorGUILayout.LabelField("app-ads.txt", string.Join(" | ", metadata.AppAdsLines));
                    EditorGUILayout.LabelField("Docs", metadata.HelpUrl);

                    using (new EditorGUILayout.HorizontalScope())
                    {
                        if (GUILayout.Button("Validate"))
                        {
                            RunValidation(metadata.Name);
                        }

                        if (GUILayout.Button("Copy app-ads line"))
                        {
                            EditorGUIUtility.systemCopyBuffer = string.Join("\n", metadata.AppAdsLines);
                            _status = $"Copied app-ads lines for {metadata.Name}";
                        }
                    }
                    EditorGUILayout.EndVertical();
                }
            }
        }

        private void DrawDiagnostics()
        {
            EditorGUILayout.LabelField("Diagnostics", EditorStyles.boldLabel);
            if (!Application.isPlaying || !ApexMediation.IsInitialized)
            {
                EditorGUILayout.HelpBox("Enter Play Mode and initialize the SDK to view live traces.", MessageType.Warning);
                return;
            }

            var traces = ApexMediation.GetTelemetryTraces();
            if (traces.Count == 0)
            {
                EditorGUILayout.HelpBox("No events recorded yet.", MessageType.Info);
                return;
            }

            foreach (var trace in traces.Reverse())
            {
                EditorGUILayout.LabelField($"[{trace.Timestamp:HH:mm:ss}] {trace.PlacementId} · {trace.Adapter} · {trace.Outcome} · {trace.Latency.TotalMilliseconds:F0} ms");
            }
        }

        private void DrawPlacements()
        {
            EditorGUILayout.LabelField("Placements", EditorStyles.boldLabel);
            var filtered = FilterPlacements(_search);

            if (_isLoading && filtered.Count == 0)
            {
                EditorGUILayout.HelpBox("Loading placement data from the console…", MessageType.Info);
            }

            if (!_isLoading && filtered.Count == 0)
            {
                EditorGUILayout.HelpBox("No placements found. Create one in the console to start mediating.", MessageType.Warning);
            }

            _placementScroll = EditorGUILayout.BeginScrollView(_placementScroll, GUILayout.Height(220));
            foreach (var placement in filtered)
            {
                EditorGUILayout.BeginVertical(EditorStyles.helpBox);
                EditorGUILayout.LabelField($"{placement.Name} ({placement.Id})", EditorStyles.largeLabel);
                EditorGUILayout.LabelField("Status", placement.Status);
                EditorGUILayout.LabelField("Format", placement.AdType);
                if (placement.FloorCpm.HasValue)
                {
                    EditorGUILayout.LabelField("Floor (CPM)", placement.FloorCpm.Value.ToString("F2"));
                }
                if (placement.TimeoutMs.HasValue)
                {
                    EditorGUILayout.LabelField("Timeout", $"{placement.TimeoutMs.Value} ms");
                }
                if (placement.RefreshInterval.HasValue)
                {
                    EditorGUILayout.LabelField("Refresh", placement.RefreshInterval.Value < 1 ? "—" : $"{placement.RefreshInterval.Value}s");
                }
                EditorGUILayout.LabelField("Adapters", placement.EnabledAdapters.Count == 0 ? "—" : string.Join(", ", placement.EnabledAdapters));
                if (placement.UpdatedAt.HasValue)
                {
                    EditorGUILayout.LabelField("Updated", placement.UpdatedAt.Value.ToLocalTime().ToString("g"));
                }
                EditorGUILayout.EndVertical();
            }
            EditorGUILayout.EndScrollView();
        }

        private void DrawConsoleStatus()
        {
            var settings = ConsoleApiSettings.Instance;
            using (new EditorGUILayout.VerticalScope(EditorStyles.helpBox))
            {
                EditorGUILayout.LabelField("Console Sync", EditorStyles.boldLabel);
                EditorGUILayout.LabelField("Base URL", settings.ApiBaseUrl);
                EditorGUILayout.LabelField("Mode", settings.UseMockData ? "Mock data" : "Live API");
                EditorGUILayout.LabelField("Token", string.IsNullOrEmpty(settings.AccessToken) ? "Not configured" : "Configured");
                if (_lastSync.HasValue)
                {
                    EditorGUILayout.LabelField("Last Sync", _lastSync.Value.ToLocalTime().ToString("g"));
                }
                if (!string.IsNullOrEmpty(_loadError))
                {
                    EditorGUILayout.HelpBox(_loadError, MessageType.Error);
                }

                using (new EditorGUILayout.HorizontalScope())
                {
                    GUI.enabled = !_isLoading;
                    if (GUILayout.Button(_isLoading ? "Syncing…" : "Refresh Console Data"))
                    {
                        RefreshConsoleData();
                    }
                    GUI.enabled = true;

                    if (GUILayout.Button("Open Settings"))
                    {
                        SettingsService.OpenProjectSettings("Project/Apex Mediation Console");
                    }
                }
            }
        }

        private async void RefreshConsoleData()
        {
            if (_apiClient == null)
            {
                _apiClient = new ConsoleApiClient();
            }

            _refreshCts?.Cancel();
            _refreshCts = new CancellationTokenSource();
            _isLoading = true;
            _loadError = null;
            Repaint();

            try
            {
                var summary = await _apiClient.FetchSummaryAsync(_refreshCts.Token);
                _placements.Clear();
                _placements.AddRange(summary.Placements);
                _adapterConfigs.Clear();
                _adapterConfigs.AddRange(summary.Adapters);
                _lastSync = DateTime.UtcNow;
            }
            catch (OperationCanceledException)
            {
                // ignored
            }
            catch (Exception ex)
            {
                _loadError = ex.Message;
            }
            finally
            {
                _isLoading = false;
                Repaint();
            }
        }

        private IReadOnlyList<ConsolePlacement> FilterPlacements(string search)
        {
            if (string.IsNullOrWhiteSpace(search))
            {
                return _placements;
            }

            return _placements
                .Where(p => p.Name.IndexOf(search, StringComparison.OrdinalIgnoreCase) >= 0
                            || p.Id.IndexOf(search, StringComparison.OrdinalIgnoreCase) >= 0)
                .ToList();
        }

        private IReadOnlyList<ConsoleAdapterConfig> FilterConsoleAdapters(string search)
        {
            if (string.IsNullOrWhiteSpace(search))
            {
                return _adapterConfigs;
            }

            return _adapterConfigs
                .Where(a => a.AdapterId.IndexOf(search, StringComparison.OrdinalIgnoreCase) >= 0)
                .ToList();
        }

        private static IEnumerable<AdapterMetadata> FilterCatalogAdapters(string search)
        {
            var all = AdapterCatalog.AllAdapters();
            if (string.IsNullOrWhiteSpace(search))
            {
                return all;
            }

            return all.Where(a => a.Name.IndexOf(search, StringComparison.OrdinalIgnoreCase) >= 0);
        }

        private void RunValidation(string network)
        {
            var result = AdapterValidationUtility.Validate(network);
            _status = result.Success
                ? $"{network} validated: {result.Message}"
                : $"{network} failed: {result.Message} (missing: {string.Join(", ", result.MissingKeys)})";
        }
    }
}
