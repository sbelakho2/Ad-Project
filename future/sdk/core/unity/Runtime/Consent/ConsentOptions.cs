using System;
using System.Collections.Generic;

namespace Apex.Mediation.Consent
{
    [Serializable]
    public sealed class ConsentOptions
    {
        public bool? GdprApplies { get; set; }
        public string? TcfString { get; set; }
        public string? GppString { get; set; }
        public string? UsPrivacyString { get; set; }
        public bool? CoppaApplies { get; set; }
        public string? AttStatus { get; set; }
        public bool? LimitAdTracking { get; set; }

        public Dictionary<string, object?> AsAdapterMap()
        {
            var map = new Dictionary<string, object?>();
            if (GdprApplies.HasValue)
            {
                map["gdpr_applies"] = GdprApplies.Value ? "1" : "0";
            }

            if (!string.IsNullOrEmpty(TcfString))
            {
                map["iab_tcf_v2"] = TcfString;
            }

            if (!string.IsNullOrEmpty(GppString))
            {
                map["iab_gpp"] = GppString;
            }

            if (!string.IsNullOrEmpty(UsPrivacyString))
            {
                map["iab_us_privacy"] = UsPrivacyString;
                map["us_privacy"] = UsPrivacyString;
            }

            if (CoppaApplies.HasValue)
            {
                map["coppa"] = CoppaApplies.Value ? "1" : "0";
            }

            if (!string.IsNullOrEmpty(AttStatus))
            {
                map["att_status"] = AttStatus;
            }

            if (LimitAdTracking.HasValue)
            {
                map["limit_ad_tracking"] = LimitAdTracking.Value;
            }

            return map;
        }

        public ConsentOptions Clone()
        {
            return (ConsentOptions)MemberwiseClone();
        }
    }
}
