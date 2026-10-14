#nullable enable
using System;
using System.Runtime.CompilerServices;
#if UNITY_2020_3_OR_NEWER
using UnityEngine;
#endif

namespace Apex.Mediation.Internal
{
    internal static class Logger
    {
        private static bool _enableDebug;

        public static void SetDebug(bool enabled) => _enableDebug = enabled;

        public static void Log(string message)
        {
            Write("[Apex] " + message);
        }

        public static void LogDebug(string message)
        {
            if (_enableDebug)
            {
                Write("[Apex][Debug] " + message);
            }
        }

        public static void LogWarning(string message)
        {
            Write("[Apex][Warn] " + message, isWarning: true);
        }

        public static void LogError(string message, Exception? exception = null)
        {
            Write("[Apex][Error] " + message + (exception != null ? " :: " + exception : string.Empty), isError: true);
        }

        private static void Write(string message, bool isWarning = false, bool isError = false)
        {
#if UNITY_2020_3_OR_NEWER
            if (isError)
            {
                Debug.LogError(message);
            }
            else if (isWarning)
            {
                Debug.LogWarning(message);
            }
            else
            {
                Debug.Log(message);
            }
#else
            Console.WriteLine(message);
#endif
        }
    }

    internal static class RedactedLogger
    {
        public static void LogInfo(string message) => Logger.Log(Redactor.Redact(message));
        public static void LogWarning(string message) => Logger.LogWarning(Redactor.Redact(message));
    }
}
