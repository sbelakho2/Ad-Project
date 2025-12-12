# Unity Duplicate Symbols Resolution Guide

## Overview

When integrating multiple ad network SDKs in Unity projects, duplicate symbol errors are a common build failure, particularly on iOS with IL2CPP. This guide provides comprehensive solutions for resolving these conflicts.

---

## Common Duplicate Symbol Scenarios

### 1. Native Library Conflicts (`.a` and `.framework`)

Multiple ad SDKs may include the same third-party libraries:
- GoogleAppMeasurement
- Firebase Analytics
- OpenSSL
- LibWebP
- Protocol Buffers

### 2. Managed DLL Conflicts

C# libraries duplicated across packages:
- Newtonsoft.Json
- Google.Protobuf
- System.* assemblies

### 3. Plugin Architecture Conflicts

Duplicate `UnityPlugin.mm` or similar bridge files.

---

## iOS Build Failures

### Symptom: Linker Error

```
duplicate symbol '_OBJC_CLASS_$_GADMobileAds' in:
    /path/to/Plugins/iOS/GoogleMobileAds.a
    /path/to/Plugins/iOS/AdMob.framework/AdMob
```

### Solution 1: Remove Duplicate Static Libraries

```csharp
// Editor/RemoveDuplicateSymbols.cs
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;
using UnityEditor.iOS.Xcode;
using System.IO;
using System.Linq;
using System.Collections.Generic;

public class RemoveDuplicateSymbols : IPostprocessBuildWithReport
{
    public int callbackOrder => 100;

    // Known duplicate libraries and which one to keep
    private static readonly Dictionary<string, string[]> DuplicateLibraries = new()
    {
        // Keep first, remove rest
        ["GoogleAppMeasurement"] = new[] {
            "Plugins/iOS/GoogleAppMeasurement.framework",
            "Plugins/iOS/Firebase/GoogleAppMeasurement.a"
        },
        ["GoogleUtilities"] = new[] {
            "Plugins/iOS/GoogleUtilities.framework",
            "Plugins/iOS/ThirdParty/GoogleUtilities.a"
        },
        ["nanopb"] = new[] {
            "Plugins/iOS/nanopb.framework",
            "Plugins/iOS/Deps/libnanopb.a"
        }
    };

    public void OnPostprocessBuild(BuildReport report)
    {
        if (report.summary.platform != BuildTarget.iOS) return;
        
        string projectPath = PBXProject.GetPBXProjectPath(report.summary.outputPath);
        var project = new PBXProject();
        project.ReadFromString(File.ReadAllText(projectPath));
        
        string targetGuid = project.GetUnityMainTargetGuid();
        string frameworkTargetGuid = project.GetUnityFrameworkTargetGuid();
        
        foreach (var entry in DuplicateLibraries)
        {
            // Keep the first library, remove others
            bool keptOne = false;
            foreach (string libPath in entry.Value)
            {
                string fullPath = Path.Combine(report.summary.outputPath, libPath);
                if (File.Exists(fullPath) || Directory.Exists(fullPath))
                {
                    if (keptOne)
                    {
                        UnityEngine.Debug.Log($"[DuplicateSymbols] Removing duplicate: {libPath}");
                        RemoveFromProject(project, targetGuid, frameworkTargetGuid, libPath);
                        
                        if (Directory.Exists(fullPath))
                            Directory.Delete(fullPath, true);
                        else
                            File.Delete(fullPath);
                    }
                    else
                    {
                        keptOne = true;
                        UnityEngine.Debug.Log($"[DuplicateSymbols] Keeping: {libPath}");
                    }
                }
            }
        }
        
        File.WriteAllText(projectPath, project.WriteToString());
    }
    
    private void RemoveFromProject(PBXProject project, string targetGuid, 
        string frameworkTargetGuid, string filePath)
    {
        string fileGuid = project.FindFileGuidByProjectPath(filePath);
        if (!string.IsNullOrEmpty(fileGuid))
        {
            project.RemoveFileFromBuild(targetGuid, fileGuid);
            project.RemoveFileFromBuild(frameworkTargetGuid, fileGuid);
            project.RemoveFile(fileGuid);
        }
    }
}
```

### Solution 2: Use Cocoapods Deduplication

```ruby
# Podfile (in iOS build output)
platform :ios, '12.0'

# Prevent duplicate pods
install! 'cocoapods', :deterministic_uuids => false

target 'UnityFramework' do
  use_frameworks! :linkage => :static
  
  # Ad SDKs - use pods instead of embedded libraries
  pod 'Google-Mobile-Ads-SDK', '~> 11.0'
  pod 'AppLovinSDK', '~> 12.0'
  pod 'UnityAds', '~> 4.9'
  pod 'FBAudienceNetwork', '~> 6.14'
  
  # Deduplicate by using shared dependencies
  pod 'GoogleUtilities', :modular_headers => true
  pod 'nanopb', :modular_headers => true
end

post_install do |installer|
  # Remove duplicate definitions
  installer.pods_project.targets.each do |target|
    target.build_configurations.each do |config|
      # Allow duplicate symbols to be resolved
      config.build_settings['OTHER_LDFLAGS'] ||= ['$(inherited)']
      config.build_settings['OTHER_LDFLAGS'] << '-ObjC'
      
      # Use MERGE_LINKED_LIBRARIES for deduplication
      config.build_settings['DEAD_CODE_STRIPPING'] = 'YES'
    end
  end
end
```

---

## Android Build Failures

### Symptom: Duplicate Class Error

```
Duplicate class com.google.android.gms.ads.MobileAds found in modules:
    play-services-ads-21.0.0.aar
    play-services-ads-lite-22.0.0.aar
```

### Solution 1: Gradle Dependency Resolution

```groovy
// mainTemplate.gradle or build.gradle
android {
    // ...
}

dependencies {
    // Force specific versions to resolve conflicts
    implementation 'com.google.android.gms:play-services-ads:23.0.0'
    
    // Exclude transitive dependencies
    implementation('com.applovin:applovin-sdk:12.4.0') {
        exclude group: 'com.google.android.gms', module: 'play-services-ads-identifier'
    }
}

configurations.all {
    resolutionStrategy {
        // Force versions
        force 'com.google.android.gms:play-services-ads:23.0.0'
        force 'com.google.android.gms:play-services-base:18.3.0'
        force 'androidx.core:core:1.12.0'
        
        // Fail on version conflicts (for debugging)
        // failOnVersionConflict()
        
        // Prefer project modules
        preferProjectModules()
    }
    
    // Exclude duplicates globally
    exclude group: 'com.google.android.gms', module: 'play-services-ads-lite'
}
```

### Solution 2: EDM4U (External Dependency Manager)

```xml
<!-- Editor/ApexMediationDependencies.xml -->
<dependencies>
  <androidPackages>
    <androidPackage spec="com.google.android.gms:play-services-ads:23.0.0">
      <repositories>
        <repository>https://maven.google.com</repository>
      </repositories>
    </androidPackage>
    
    <!-- Prevent other plugins from pulling different versions -->
    <androidPackage spec="com.google.android.gms:play-services-base:18.3.0">
      <repositories>
        <repository>https://maven.google.com</repository>
      </repositories>
    </androidPackage>
  </androidPackages>

  <iosPods>
    <iosPod name="Google-Mobile-Ads-SDK" version="~> 11.0" />
    <iosPod name="GoogleUtilities" version="~> 7.13" />
  </iosPods>
</dependencies>
```

```csharp
// Editor/DependencyResolver.cs
using UnityEditor;
using System;

[InitializeOnLoad]
public static class DependencyResolver
{
    static DependencyResolver()
    {
        // Force resolution on load
        EditorApplication.delayCall += () =>
        {
            try
            {
                // Invoke EDM4U resolution
                var resolverType = Type.GetType(
                    "GooglePlayServices.PlayServicesResolver, Google.JarResolver");
                if (resolverType != null)
                {
                    var resolveMethod = resolverType.GetMethod("Resolve",
                        System.Reflection.BindingFlags.Public | 
                        System.Reflection.BindingFlags.Static,
                        null, new Type[0], null);
                    resolveMethod?.Invoke(null, null);
                }
            }
            catch (Exception e)
            {
                UnityEngine.Debug.LogWarning($"Dependency resolution failed: {e.Message}");
            }
        };
    }
}
```

---

## Managed DLL Conflicts

### Symptom: Assembly Conflict

```
error CS0433: The type 'JsonConvert' exists in both 
    'Newtonsoft.Json, Version=12.0.0.0' and 
    'Newtonsoft.Json, Version=13.0.0.0'
```

### Solution: Assembly Definition Files

```csharp
// Editor/DLLConflictResolver.cs
using UnityEditor;
using UnityEngine;
using System.IO;
using System.Linq;
using System.Collections.Generic;

public class DLLConflictResolver : AssetPostprocessor
{
    // DLLs that should have only one version
    private static readonly Dictionary<string, string> PreferredVersions = new()
    {
        ["Newtonsoft.Json"] = "Assets/Plugins/Newtonsoft.Json.dll",
        ["Google.Protobuf"] = "Assets/Plugins/Google.Protobuf.dll",
        ["System.Memory"] = "Assets/Plugins/System.Memory.dll"
    };
    
    private static void OnPostprocessAllAssets(
        string[] importedAssets,
        string[] deletedAssets,
        string[] movedAssets,
        string[] movedFromAssetPaths)
    {
        foreach (string asset in importedAssets)
        {
            if (!asset.EndsWith(".dll")) continue;
            
            string dllName = Path.GetFileNameWithoutExtension(asset);
            
            if (PreferredVersions.TryGetValue(dllName, out string preferredPath))
            {
                if (asset != preferredPath && File.Exists(preferredPath))
                {
                    Debug.LogWarning($"[DLLConflict] Disabling duplicate: {asset} " +
                        $"(keeping {preferredPath})");
                    
                    // Disable the plugin
                    var importer = AssetImporter.GetAtPath(asset) as PluginImporter;
                    if (importer != null)
                    {
                        importer.SetCompatibleWithAnyPlatform(false);
                        importer.SetCompatibleWithEditor(false);
                        importer.SaveAndReimport();
                    }
                }
            }
        }
    }
    
    [MenuItem("ApexMediation/Resolve DLL Conflicts")]
    public static void ResolveConflicts()
    {
        var allDlls = AssetDatabase.FindAssets("t:DefaultAsset")
            .Select(AssetDatabase.GUIDToAssetPath)
            .Where(p => p.EndsWith(".dll"))
            .ToList();
        
        var dllGroups = allDlls
            .GroupBy(p => Path.GetFileNameWithoutExtension(p))
            .Where(g => g.Count() > 1);
        
        foreach (var group in dllGroups)
        {
            Debug.Log($"[DLLConflict] Found duplicates for {group.Key}:");
            foreach (string path in group)
            {
                Debug.Log($"  - {path}");
            }
            
            // Keep the first, disable rest
            bool first = true;
            foreach (string path in group)
            {
                if (first)
                {
                    first = false;
                    continue;
                }
                
                var importer = AssetImporter.GetAtPath(path) as PluginImporter;
                if (importer != null)
                {
                    importer.SetCompatibleWithAnyPlatform(false);
                    importer.SetCompatibleWithEditor(false);
                    importer.SaveAndReimport();
                    Debug.Log($"[DLLConflict] Disabled: {path}");
                }
            }
        }
        
        Debug.Log("[DLLConflict] Resolution complete");
    }
}
```

---

## IL2CPP Specific Issues

### Symptom: IL2CPP Stripping Removes Symbols

```
NotSupportedException: IL2CPP does not support marshaling delegates 
that point to instance methods to native code.
```

### Solution: Preserve Required Types

```xml
<!-- link.xml in Assets folder -->
<linker>
    <!-- Preserve Ad SDK types -->
    <assembly fullname="ApexMediationSDK">
        <type fullname="ApexMediation.*" preserve="all"/>
    </assembly>
    
    <!-- Preserve adapter types -->
    <assembly fullname="ApexMediationAdapters">
        <type fullname="ApexMediation.Adapters.*" preserve="all"/>
    </assembly>
    
    <!-- Preserve JSON serialization -->
    <assembly fullname="Newtonsoft.Json">
        <type fullname="Newtonsoft.Json.*" preserve="all"/>
    </assembly>
    
    <!-- Preserve reflection for callbacks -->
    <assembly fullname="mscorlib">
        <type fullname="System.Reflection.*" preserve="all"/>
    </assembly>
    
    <!-- Common ad network types -->
    <assembly fullname="GoogleMobileAds">
        <type fullname="GoogleMobileAds.*" preserve="all"/>
    </assembly>
    
    <assembly fullname="UnityEngine.Advertisements">
        <type fullname="UnityEngine.Advertisements.*" preserve="all"/>
    </assembly>
</linker>
```

### Managed Stripping Level

```csharp
// Editor/IL2CPPSettings.cs
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;

public class IL2CPPSettings : IPreprocessBuildWithReport
{
    public int callbackOrder => 0;
    
    public void OnPreprocessBuild(BuildReport report)
    {
        // Use Minimal stripping to preserve more types
        PlayerSettings.SetManagedStrippingLevel(
            report.summary.platformGroup, 
            ManagedStrippingLevel.Minimal);
        
        // For iOS, ensure proper IL2CPP options
        if (report.summary.platform == BuildTarget.iOS)
        {
            PlayerSettings.SetScriptingBackend(
                BuildTargetGroup.iOS, 
                ScriptingImplementation.IL2CPP);
            
            // Include debug info for better error messages
            EditorUserBuildSettings.iOSBuildConfigType = iOSBuildType.Debug;
        }
    }
}
```

---

## Plugin Import Settings

### Automated Plugin Configuration

```csharp
// Editor/PluginImportFixer.cs
using UnityEditor;
using UnityEngine;
using System.IO;

public class PluginImportFixer : AssetPostprocessor
{
    void OnPreprocessAsset()
    {
        if (!assetPath.Contains("Plugins")) return;
        
        var pluginImporter = assetImporter as PluginImporter;
        if (pluginImporter == null) return;
        
        // iOS native libraries
        if (assetPath.Contains("Plugins/iOS"))
        {
            ConfigureIOSPlugin(pluginImporter);
        }
        // Android libraries
        else if (assetPath.Contains("Plugins/Android"))
        {
            ConfigureAndroidPlugin(pluginImporter);
        }
    }
    
    private void ConfigureIOSPlugin(PluginImporter importer)
    {
        string filename = Path.GetFileName(assetPath);
        
        // Disable duplicate libraries
        if (IsDuplicateIOSLibrary(filename))
        {
            Debug.Log($"[PluginFix] Disabling duplicate iOS library: {assetPath}");
            importer.SetCompatibleWithPlatform(BuildTarget.iOS, false);
            return;
        }
        
        // Configure .a and .framework files
        if (assetPath.EndsWith(".a") || assetPath.EndsWith(".framework"))
        {
            importer.SetCompatibleWithPlatform(BuildTarget.iOS, true);
            importer.SetCompatibleWithPlatform(BuildTarget.tvOS, true);
            importer.SetPlatformData(BuildTarget.iOS, "AddToEmbeddedBinaries", "false");
        }
    }
    
    private void ConfigureAndroidPlugin(PluginImporter importer)
    {
        string filename = Path.GetFileName(assetPath);
        
        // Disable duplicate AARs
        if (IsDuplicateAndroidLibrary(filename))
        {
            Debug.Log($"[PluginFix] Disabling duplicate Android library: {assetPath}");
            importer.SetCompatibleWithPlatform(BuildTarget.Android, false);
            return;
        }
        
        if (assetPath.EndsWith(".aar") || assetPath.EndsWith(".jar"))
        {
            importer.SetCompatibleWithPlatform(BuildTarget.Android, true);
        }
    }
    
    private bool IsDuplicateIOSLibrary(string filename)
    {
        // Known duplicates - add as discovered
        string[] duplicates = {
            "libGoogleAppMeasurement.a",
            "libGoogleUtilities.a",
            "libnanopb.a"
        };
        
        foreach (string dup in duplicates)
        {
            if (filename.Contains(dup)) return true;
        }
        return false;
    }
    
    private bool IsDuplicateAndroidLibrary(string filename)
    {
        // Known duplicates
        string[] duplicates = {
            "play-services-ads-lite",
            "play-services-basement-legacy"
        };
        
        foreach (string dup in duplicates)
        {
            if (filename.Contains(dup)) return true;
        }
        return false;
    }
}
```

---

## Debugging Duplicate Symbols

### iOS: Identify Duplicates

```bash
# List all symbols in a static library
nm -gU /path/to/Library.a | grep "OBJC_CLASS"

# Find duplicates across libraries
find . -name "*.a" -exec nm -gU {} \; 2>/dev/null | sort | uniq -d

# Check framework symbols
nm -gU /path/to/Framework.framework/Framework | head -50
```

### Android: Identify Duplicates

```bash
# Extract AAR and check classes
unzip -l library.aar | grep "\.class"

# Find duplicate classes
find . -name "*.aar" -exec unzip -l {} \; | grep "\.class" | sort | uniq -d
```

### Unity Editor Script

```csharp
// Editor/DuplicateSymbolFinder.cs
using UnityEditor;
using UnityEngine;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Diagnostics;

public class DuplicateSymbolFinder : EditorWindow
{
    private Vector2 scrollPos;
    private List<string> results = new();
    
    [MenuItem("ApexMediation/Find Duplicate Symbols")]
    public static void ShowWindow()
    {
        GetWindow<DuplicateSymbolFinder>("Duplicate Symbols");
    }
    
    private void OnGUI()
    {
        if (GUILayout.Button("Scan iOS Libraries"))
        {
            ScanIOSLibraries();
        }
        
        if (GUILayout.Button("Scan Android Libraries"))
        {
            ScanAndroidLibraries();
        }
        
        GUILayout.Space(10);
        
        scrollPos = EditorGUILayout.BeginScrollView(scrollPos);
        foreach (string result in results)
        {
            EditorGUILayout.LabelField(result);
        }
        EditorGUILayout.EndScrollView();
    }
    
    private void ScanIOSLibraries()
    {
        results.Clear();
        
        var libraries = Directory.GetFiles(
            Application.dataPath, "*.a", SearchOption.AllDirectories)
            .Concat(Directory.GetDirectories(
                Application.dataPath, "*.framework", SearchOption.AllDirectories));
        
        var symbolMap = new Dictionary<string, List<string>>();
        
        foreach (string lib in libraries)
        {
#if UNITY_EDITOR_OSX
            var psi = new ProcessStartInfo
            {
                FileName = "nm",
                Arguments = $"-gU \"{lib}\"",
                UseShellExecute = false,
                RedirectStandardOutput = true,
                CreateNoWindow = true
            };
            
            try
            {
                using var process = Process.Start(psi);
                string output = process.StandardOutput.ReadToEnd();
                process.WaitForExit();
                
                foreach (string line in output.Split('\n'))
                {
                    if (line.Contains("OBJC_CLASS"))
                    {
                        string symbol = line.Trim();
                        if (!symbolMap.ContainsKey(symbol))
                            symbolMap[symbol] = new List<string>();
                        symbolMap[symbol].Add(Path.GetFileName(lib));
                    }
                }
            }
            catch { }
#endif
        }
        
        foreach (var kvp in symbolMap.Where(x => x.Value.Count > 1))
        {
            results.Add($"DUPLICATE: {kvp.Key}");
            foreach (string lib in kvp.Value)
            {
                results.Add($"  - {lib}");
            }
        }
        
        if (results.Count == 0)
        {
            results.Add("No duplicate symbols found!");
        }
    }
    
    private void ScanAndroidLibraries()
    {
        results.Clear();
        
        var aars = Directory.GetFiles(
            Application.dataPath, "*.aar", SearchOption.AllDirectories);
        
        results.Add($"Found {aars.Length} AAR files:");
        foreach (string aar in aars)
        {
            results.Add($"  - {Path.GetFileName(aar)}");
        }
        
        // Group by library name (ignoring version)
        var groups = aars
            .Select(a => new {
                Path = a,
                Name = ExtractLibraryName(Path.GetFileName(a))
            })
            .GroupBy(x => x.Name)
            .Where(g => g.Count() > 1);
        
        results.Add("");
        results.Add("Potential duplicates:");
        foreach (var group in groups)
        {
            results.Add($"  {group.Key}:");
            foreach (var item in group)
            {
                results.Add($"    - {Path.GetFileName(item.Path)}");
            }
        }
    }
    
    private string ExtractLibraryName(string filename)
    {
        // Remove version numbers: "library-1.2.3.aar" -> "library"
        var parts = filename.Replace(".aar", "").Split('-');
        return string.Join("-", parts.TakeWhile(p => !char.IsDigit(p[0])));
    }
}
```

---

## Prevention Strategies

### 1. Use Package Manager Where Possible

Prefer Unity Package Manager packages over manual imports:
```json
// Packages/manifest.json
{
  "dependencies": {
    "com.google.ads.mobile": "9.0.0",
    "com.unity.ads": "4.4.2"
  }
}
```

### 2. Version Lock Dependencies

```xml
<!-- Dependencies.xml -->
<dependencies>
  <androidPackages>
    <!-- Lock all transitive dependencies -->
    <androidPackage spec="com.google.android.gms:play-services-ads:23.0.0" />
    <androidPackage spec="com.google.android.gms:play-services-base:18.3.0" />
    <androidPackage spec="androidx.core:core:1.12.0" />
  </androidPackages>
</dependencies>
```

### 3. Regular Dependency Audit

```csharp
// Editor/DependencyAudit.cs
[MenuItem("ApexMediation/Audit Dependencies")]
public static void AuditDependencies()
{
    var report = new System.Text.StringBuilder();
    report.AppendLine("=== Dependency Audit ===\n");
    
    // Count plugins by type
    var dlls = AssetDatabase.FindAssets("t:DefaultAsset")
        .Select(AssetDatabase.GUIDToAssetPath)
        .Where(p => p.EndsWith(".dll")).ToList();
    var aars = AssetDatabase.FindAssets("t:DefaultAsset")
        .Select(AssetDatabase.GUIDToAssetPath)
        .Where(p => p.EndsWith(".aar")).ToList();
    var frameworks = Directory.GetDirectories(
        Application.dataPath, "*.framework", SearchOption.AllDirectories);
    
    report.AppendLine($"DLLs: {dlls.Count}");
    report.AppendLine($"AARs: {aars.Count}");
    report.AppendLine($"Frameworks: {frameworks.Length}");
    
    UnityEngine.Debug.Log(report.ToString());
}
```

---

## Quick Reference: Common Conflicts & Solutions

| Conflict | Root Cause | Solution |
|----------|------------|----------|
| GoogleAppMeasurement duplicate | Firebase + AdMob both include it | Use EDM4U with single pods |
| play-services-ads versions | Multiple adapters specify different versions | Force version in gradle |
| Newtonsoft.Json duplicate | Multiple packages embed it | Disable extras via PluginImporter |
| nanopb symbols | Embedded in multiple frameworks | Remove duplicates post-build |
| protobuf duplicate | gRPC + other libs | Use single protobuf source |

---

## Support Resources

- Unity Forum: [IL2CPP Troubleshooting](https://forum.unity.com/forums/il2cpp.436/)
- EDM4U: [External Dependency Manager](https://github.com/googlesamples/unity-jar-resolver)
- [Unity iOS Plugin Guide](https://docs.unity3d.com/Manual/PluginsForIOS.html)
- [Unity Android Plugin Guide](https://docs.unity3d.com/Manual/PluginsForAndroid.html)

For additional help, contact sdk-support@apexmediation.ee with:
1. Full build log
2. List of ad SDKs/versions
3. Unity version
4. Target platform(s)
