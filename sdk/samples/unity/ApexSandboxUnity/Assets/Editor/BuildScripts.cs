// Editor build helpers to export Android/iOS builds and stamp SDK versions
using System;
using System.IO;
using UnityEditor;
using UnityEditor.Build.Reporting;
using UnityEngine;

public static class ApexBuildScripts
{
    private const string ScenePath = "Assets/Scenes/Main.unity";
    private const string VersionsResourcePath = "Assets/Resources/sdk_versions.json";

    [MenuItem("ApexSandbox/Export Android (Gradle)")]
    public static void ExportAndroidMenu() => ExportAndroid();

    [MenuItem("ApexSandbox/Export iOS (Xcode)")]
    public static void ExportiOSMenu() => ExportiOS();

    public static void ExportAndroid(string outDir = "Builds/Android")
    {
        EnsureSceneExists();
        Directory.CreateDirectory(outDir);
        StampVersions();
        var options = new BuildPlayerOptions
        {
            scenes = new[] { ScenePath },
            locationPathName = Path.Combine(outDir, "ApexSandboxUnity.apk"),
            target = BuildTarget.Android,
            options = BuildOptions.Development
        };
        var report = BuildPipeline.BuildPlayer(options);
        PostReport(report);
    }

    public static void ExportiOS(string outDir = "Builds/iOS")
    {
        EnsureSceneExists();
        Directory.CreateDirectory(outDir);
        StampVersions();
        var options = new BuildPlayerOptions
        {
            scenes = new[] { ScenePath },
            locationPathName = outDir,
            target = BuildTarget.iOS,
            options = BuildOptions.Development
        };
        var report = BuildPipeline.BuildPlayer(options);
        PostReport(report);
    }

    private static void EnsureSceneExists()
    {
        if (!File.Exists(ScenePath))
        {
            throw new FileNotFoundException($"Required scene not found: {ScenePath}");
        }
    }

    private static void StampVersions()
    {
        // These should match native SDK versions; keep in sync with the repo SDKs
        var payload = new
        {
            unityBridge = "1.0.0",
            iosSdk = "1.0.0",      // sdk/core/ios/Sources/MediationSDK.swift sdkVersionValue
            androidSdk = "1.0.0"
        };
        var json = JsonUtility.ToJson(payload, prettyPrint: true);
        Directory.CreateDirectory(Path.GetDirectoryName(VersionsResourcePath) ?? "Assets/Resources");
        File.WriteAllText(VersionsResourcePath, json);
        AssetDatabase.ImportAsset(VersionsResourcePath);
    }

    private static void PostReport(BuildReport report)
    {
        if (report.summary.result == BuildResult.Succeeded)
        {
            Debug.Log($"[Build] Succeeded: {report.summary.outputPath} size={report.summary.totalSize} bytes");
        }
        else
        {
            Debug.LogError($"[Build] Failed: {report.summary.result}");
        }
    }
}
