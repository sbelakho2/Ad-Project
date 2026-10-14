using System;
using System.Collections.Concurrent;

namespace Apex.Mediation.Internal
{
#if UNITY_2020_3_OR_NEWER
    using UnityEngine;

    /// <summary>
    /// Dispatches work onto Unity's main thread.
    /// </summary>
    internal sealed class UnityMainThread : MonoBehaviour
    {
        private static readonly ConcurrentQueue<Action> Pending = new();
        private static UnityMainThread? _instance;

        public static void Ensure()
        {
            if (_instance != null)
            {
                return;
            }

            var go = new GameObject("ApexMainThread");
            DontDestroyOnLoad(go);
            _instance = go.AddComponent<UnityMainThread>();
        }

        public static void Dispatch(Action action)
        {
            if (action == null)
            {
                return;
            }

            Pending.Enqueue(action);
        }

        private void Update()
        {
            while (Pending.TryDequeue(out var action))
            {
                action();
            }
        }
    }
#else
    internal static class UnityMainThread
    {
        public static void Ensure()
        {
        }

        public static void Dispatch(Action action)
        {
            action?.Invoke();
        }
    }
#endif
}
