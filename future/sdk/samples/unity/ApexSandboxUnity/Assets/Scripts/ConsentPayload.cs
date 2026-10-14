// Copyright (c) 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ.
using System;
using System.Runtime.InteropServices;

namespace ApexSandboxUnity
{
    [Serializable]
    public struct ConsentPayload
    {
        public bool gdprApplies; // if true, tcfString may be populated
        public string tcfString; // IAB TCF v2
        public bool ccpaOptOut;  // CCPA Do Not Sell
        public bool coppa;       // COPPA child-directed
        public bool testMode;    // propagate test mode flag

        public static ConsentPayload Default() => new ConsentPayload
        {
            gdprApplies = false,
            tcfString = null,
            ccpaOptOut = false,
            coppa = false,
            testMode = true
        };
    }
}
