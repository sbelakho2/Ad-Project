#if UNITY_2020_3_OR_NEWER
using System;
using UnityEngine;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Hosts a native banner view layered on top of the Unity canvas.
    /// </summary>
    public sealed class BannerHost : MonoBehaviour
    {
        [SerializeField]
        private string placementId = "banner_default";

        private IntPtr _nativeHandle = IntPtr.Zero;

        private void OnEnable()
        {
            CreateNativeView();
        }

        private void OnDisable()
        {
            DestroyNativeView();
        }

        private void OnApplicationPause(bool paused)
        {
            if (paused)
            {
                DestroyNativeView();
            }
            else
            {
                CreateNativeView();
            }
        }

        private void CreateNativeView()
        {
            if (string.IsNullOrEmpty(placementId) || !ApexMediation.IsInitialized)
            {
                return;
            }

            if (_nativeHandle == IntPtr.Zero)
            {
                _nativeHandle = (IntPtr)GetInstanceID();
            }

            ApexMediation.AttachBanner(placementId, _nativeHandle);
        }

        private void DestroyNativeView()
        {
            if (_nativeHandle == IntPtr.Zero)
            {
                return;
            }

            ApexMediation.DestroyBanner(placementId);
        }
    }
}
#endif
