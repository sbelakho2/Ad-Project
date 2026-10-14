using UnityEditor;
using UnityEngine;

namespace Apex.Mediation.Editor.Console
{
    [FilePath("ProjectSettings/ApexMediationConsoleSettings.asset", FilePathAttribute.Location.ProjectFolder)]
    internal sealed class ConsoleApiSettings : ScriptableSingleton<ConsoleApiSettings>
    {
        private const string DefaultBaseUrl = "https://console.apexmediation.ee/api/v1";

        [SerializeField] private string apiBaseUrl = DefaultBaseUrl;
        [SerializeField] private string accessToken = string.Empty;
        [SerializeField] private bool useMockData = true;
        [SerializeField] private int requestTimeoutSeconds = 15;

        public static ConsoleApiSettings Instance => instance;

        public string ApiBaseUrl => string.IsNullOrWhiteSpace(apiBaseUrl) ? DefaultBaseUrl : apiBaseUrl.TrimEnd('/');
        public string AccessToken => accessToken?.Trim() ?? string.Empty;
        public bool UseMockData => useMockData;
        public int RequestTimeoutSeconds => Mathf.Clamp(requestTimeoutSeconds, 5, 120);

        public void Update(string baseUrl, string token, bool mockData, int timeoutSeconds)
        {
            apiBaseUrl = string.IsNullOrWhiteSpace(baseUrl) ? DefaultBaseUrl : baseUrl.TrimEnd('/');
            accessToken = token ?? string.Empty;
            useMockData = mockData;
            requestTimeoutSeconds = Mathf.Clamp(timeoutSeconds, 5, 120);
            Save(true);
        }

        private void OnDisable()
        {
            Save(true);
        }

        [SettingsProvider]
        private static SettingsProvider CreateProvider()
        {
            var provider = new SettingsProvider("Project/Apex Mediation Console", SettingsScope.Project)
            {
                label = "Apex Mediation Console",
                guiHandler = _ => DrawSettingsGUI()
            };
            return provider;
        }

        private static void DrawSettingsGUI()
        {
            var settings = Instance;
            EditorGUILayout.LabelField("Console API", EditorStyles.boldLabel);
            EditorGUILayout.HelpBox("Configure how the Unity Editor connects to your Apex console APIs. Tokens are stored locally under ProjectSettings and should not be committed to source control.", MessageType.Info);

            EditorGUI.BeginChangeCheck();
            var baseUrl = EditorGUILayout.TextField("Base URL", settings.apiBaseUrl);
            var token = EditorGUILayout.PasswordField("Access Token", settings.accessToken);
            var timeout = EditorGUILayout.IntSlider("Timeout (seconds)", settings.requestTimeoutSeconds, 5, 120);
            var mockData = EditorGUILayout.ToggleLeft("Use mock data when offline", settings.useMockData);

            if (EditorGUI.EndChangeCheck())
            {
                settings.Update(baseUrl, token, mockData, timeout);
            }

            using (new EditorGUILayout.HorizontalScope())
            {
                if (GUILayout.Button("Copy Docs URL"))
                {
                    EditorGUIUtility.systemCopyBuffer = "https://apexmediation.ee/docs/unity/console-api";
                }

                if (GUILayout.Button("Open Credential Wizard"))
                {
                    global::Apex.Mediation.Editor.AdapterCredentialWizard.ShowWindow();
                }
            }
        }
    }
}
