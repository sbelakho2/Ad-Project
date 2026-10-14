namespace Apex.Mediation.Platforms
{
    internal static class PlatformBridgeFactory
    {
        public static IPlatformBridge Create()
        {
#if UNITY_ANDROID && !UNITY_EDITOR
            return new Android.AndroidPlatformBridge();
#elif UNITY_IOS && !UNITY_EDITOR
            return new iOS.IosPlatformBridge();
#else
            return new MockPlatformBridge();
#endif
        }
    }
}
