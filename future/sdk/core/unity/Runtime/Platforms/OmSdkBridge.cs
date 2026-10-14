using UnityEngine;

namespace Apex.Mediation.Platforms
{
    /// Thin Unity bridge that delegates to native OMSDK helpers when available.
    public static class OmSdkBridge
    {
#if UNITY_ANDROID && !UNITY_EDITOR
        private static AndroidJavaClass _helper;
        private static AndroidJavaObject _sessionHandle;
#endif

        public static bool IsAvailable()
        {
#if UNITY_ANDROID && !UNITY_EDITOR
            try
            {
                var helper = GetHelper();
                return helper != null && helper.CallStatic<bool>("isAvailable");
            }
            catch
            {
                return false;
            }
#elif UNITY_IOS && !UNITY_EDITOR
            // Native binding stub; add when iOS bridge is exported.
            return false;
#else
            return false;
#endif
        }

        public static void StartSession(bool isVideo)
        {
#if UNITY_ANDROID && !UNITY_EDITOR
            try
            {
                var helper = GetHelper();
                if (helper == null) return;

                using (var unityPlayer = new AndroidJavaClass("com.unity3d.player.UnityPlayer"))
                {
                    var activity = unityPlayer.GetStatic<AndroidJavaObject>("currentActivity");
                    var window = activity != null ? activity.Call<AndroidJavaObject>("getWindow") : null;
                    var decorView = window != null ? window.Call<AndroidJavaObject>("getDecorView") : null;
                    if (decorView == null) return;

                    using (var obstructions = new AndroidJavaObject("java.util.ArrayList"))
                    {
                        _sessionHandle = helper.CallStatic<AndroidJavaObject>("startSession", decorView, isVideo, obstructions);
                    }
                }
            }
            catch
            {
                // Swallow; OMSDK is optional.
            }
#endif
        }

        public static void FinishSession()
        {
#if UNITY_ANDROID && !UNITY_EDITOR
            try
            {
                var helper = GetHelper();
                if (helper == null) return;
                helper.CallStatic("finishSession", _sessionHandle);
                _sessionHandle = null;
            }
            catch
            {
                // Ignore cleanup errors
            }
#endif
        }

#if UNITY_ANDROID && !UNITY_EDITOR
        private static AndroidJavaClass GetHelper()
        {
            if (_helper == null)
            {
                _helper = new AndroidJavaClass("com.rivalapexmediation.sdk.measurement.OmSdkHelper");
            }
            return _helper;
        }
#endif
    }
}
