// Copyright (c) 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ.
// Unity sandbox shared enums
using System;

namespace ApexSandboxUnity
{
    public enum AdFormat
    {
        Interstitial,
        Rewarded,
        Banner
    }

    public enum AdError
    {
        None = 0,
        NoFill = 1,
        Timeout = 2,
        Network = 3,
        InvalidPlacement = 4,
        Unknown = 99
    }

    public static class AdErrorExtensions
    {
        public static string ToMessage(this AdError error)
        {
            switch (error)
            {
                case AdError.NoFill: return "no_fill";
                case AdError.Timeout: return "timeout";
                case AdError.Network: return "network";
                case AdError.InvalidPlacement: return "invalid_placement";
                case AdError.None: return "none";
                default: return "unknown";
            }
        }
    }
}
