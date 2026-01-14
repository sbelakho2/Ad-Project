using UnityEngine;
using UnityEngine.UI;

public class SceneController : MonoBehaviour
{
    public Text StatusText;

    void Start()
    {
        if (!IsEmulatorOrSimulator())
        {
            if (StatusText != null) StatusText.text = "Emulator/Simulator Only. Exiting...";
            // Give a moment to render the message, then quit.
            Invoke(nameof(QuitApp), 0.5f);
        }
        else
        {
            if (StatusText != null) StatusText.text = "SDK: not initialized";
        }
    }

    void QuitApp()
    {
#if UNITY_EDITOR
        UnityEditor.EditorApplication.isPlaying = false;
#else
        Application.Quit();
#endif
    }

    public void Initialize()
    {
        if (StatusText != null) StatusText.text = "SDK: initialized (stub)";
    }

    public void LoadInterstitial()
    {
        if (StatusText != null) StatusText.text = "Interstitial: loaded (stub)";
    }

    public void ShowInterstitial()
    {
        if (StatusText != null) StatusText.text = "Interstitial: shown (stub)";
    }

    public void LoadRewarded()
    {
        if (StatusText != null) StatusText.text = "Rewarded: loaded (stub)";
    }

    public void ShowRewarded()
    {
        if (StatusText != null) StatusText.text = "Rewarded: shown (stub)";
    }

    bool IsEmulatorOrSimulator()
    {
#if UNITY_ANDROID && !UNITY_EDITOR
        try
        {
            using (var buildClass = new AndroidJavaClass("android.os.Build"))
            {
                string brand = buildClass.GetStatic<string>("BRAND");
                string device = buildClass.GetStatic<string>("DEVICE");
                string product = buildClass.GetStatic<string>("PRODUCT");
                string hardware = buildClass.GetStatic<string>("HARDWARE");
                string model = buildClass.GetStatic<string>("MODEL");
                string fingerprint = buildClass.GetStatic<string>("FINGERPRINT");

                if ((brand != null && (brand.StartsWith("generic") || brand.StartsWith("google"))) ||
                    (device != null && device.StartsWith("generic")) ||
                    (product != null && (product.Contains("sdk") || product.Contains("emulator") || product.Contains("simulator"))) ||
                    (hardware != null && (hardware.Contains("goldfish") || hardware.Contains("ranchu") || hardware.Contains("qemu"))) ||
                    (model != null && model.Contains("Android SDK built for")) ||
                    (fingerprint != null && (fingerprint.StartsWith("generic") || fingerprint.StartsWith("unknown"))))
                {
                    return true;
                }
            }
        }
        catch { }
        return false;
#elif UNITY_IOS && !UNITY_EDITOR
        // Best-effort iOS Simulator detection: environment variable typically present when running in Simulator.
        var simName = System.Environment.GetEnvironmentVariable("SIMULATOR_DEVICE_NAME");
        if (!string.IsNullOrEmpty(simName)) return true;
        // Fallback heuristic: many simulators report deviceModel starting with "x86" on Intel Macs.
        if (SystemInfo.deviceModel != null && SystemInfo.deviceModel.StartsWith("x86")) return true;
        return false;
#else
        // In Editor or other platforms, allow by default.
        return true;
#endif
    }
}
