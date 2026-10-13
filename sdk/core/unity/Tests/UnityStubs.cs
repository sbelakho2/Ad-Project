// UnityStubs.cs — minimal UnityEngine surface for tier-1 (no-editor) test runs.
// Tier 1 runs the net8 xunit project with UNITY_RUNNER=true. Files that need the real
// editor runtime (ApexMediationEntryPoint.cs, Core/MediationDebuggerOverlay.cs) are
// excluded in the csproj and covered by tier 2 (Unity Test Framework, licensed runner).
// This file provides just enough API for the editor-independent runtime code to compile
// and execute on the plain .NET runtime.
using System;
using System.Collections.Generic;

namespace UnityEngine
{
    public class Object
    {
        public string name = string.Empty;
        public static T FindObjectOfType<T>() where T : Object => null;
        public static void DontDestroyOnLoad(Object target) { }
        public static void Destroy(Object target) { }
    }

    public class GameObject : Object
    {
        public GameObject() { }
        public GameObject(string objectName) { name = objectName; }
        public T AddComponent<T>() where T : MonoBehaviour, new() => new T();
        public void SetActive(bool value) { }
    }

    public class MonoBehaviour : Object
    {
        public GameObject gameObject { get; } = new GameObject();
        public T GetComponent<T>() => default!;
    }

    public enum GraphicsDeviceType
    {
        Unknown, OpenGLES2, OpenGLES3, OpenGLCore, OpenGL, Metal, Vulkan, Direct3D11, Direct3D12, Null,
    }

    public static class SystemInfo
    {
        public static int processorCount => Environment.ProcessorCount;
        public static int systemMemorySize => 4096;
        public static GraphicsDeviceType graphicsDeviceType => GraphicsDeviceType.Null;
    }

    public static class Debug
    {
        public static void Log(object message) { }
        public static void LogWarning(object message) { }
        public static void LogError(object message) { }
    }

    public enum RuntimePlatform
    {
        OSXEditor, OSXPlayer, WindowsPlayer, WindowsEditor,
        IPhonePlayer, Android, tvOS, WebGLPlayer,
        LinuxPlayer, LinuxEditor,
    }

    public static class Application
    {
        public static bool isEditor => false;
        public static string dataPath => AppContext.BaseDirectory;
        public static string streamingAssetsPath => System.IO.Path.Combine(AppContext.BaseDirectory, "StreamingAssets");
        public static RuntimePlatform platform => RuntimePlatform.LinuxPlayer;
        public static Networking.NetworkReachability internetReachability =>
            Networking.NetworkReachability.ReachableViaLocalAreaNetwork;
    }

    public static class PlayerPrefs
    {
        private static readonly Dictionary<string, string> Store = new();
        public static bool HasKey(string key) => Store.ContainsKey(key);
        public static string GetString(string key, string defaultValue = "") => Store.TryGetValue(key, out var v) ? v : defaultValue;
        public static void SetString(string key, string value) { Store[key] = value; }
        public static void DeleteKey(string key) { Store.Remove(key); }
        public static void DeleteAll() { Store.Clear(); }
        public static void Save() { }
    }

    public static class Time
    {
        private static readonly DateTime Start = DateTime.UtcNow;
        public static float unscaledTime => (float)(DateTime.UtcNow - Start).TotalSeconds;
        public static float unscaledDeltaTime => 0.016f;
        public static float realtimeSinceStartup => unscaledTime;
    }

    public static class Screen
    {
        public static int width => 1920;
        public static int height => 1080;
    }
}

namespace UnityEngine.Networking
{
    public enum NetworkReachability
    {
        NotReachable = 0,
        ReachableViaCarrierDataNetwork = 1,
        ReachableViaLocalAreaNetwork = 2,
    }
}
