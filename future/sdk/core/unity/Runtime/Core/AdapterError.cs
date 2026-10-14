using System;

namespace Apex.Mediation.Core
{
    internal enum AdapterErrorCode
    {
        NoFill,
        Timeout,
        NetworkError,
        Status4xx,
        Status5xx,
        BelowFloor,
        CircuitOpen,
        Config,
        Error
    }

    internal readonly struct AdapterError
    {
        public AdapterError(AdapterErrorCode code, string detail, string? vendorCode = null, bool recoverable = false)
        {
            Code = code;
            Detail = detail;
            VendorCode = vendorCode;
            Recoverable = recoverable;
        }

        public AdapterErrorCode Code { get; }
        public string Detail { get; }
        public string? VendorCode { get; }
        public bool Recoverable { get; }

        public string NormalizedReason()
        {
            return Code switch
            {
                AdapterErrorCode.NoFill => "no_fill",
                AdapterErrorCode.Timeout => "timeout",
                AdapterErrorCode.NetworkError => "network_error",
                AdapterErrorCode.Status4xx => "status_4xx",
                AdapterErrorCode.Status5xx => "status_5xx",
                AdapterErrorCode.BelowFloor => "below_floor",
                AdapterErrorCode.CircuitOpen => "circuit_open",
                AdapterErrorCode.Config => "config",
                _ => "error"
            };
        }

        public static AdapterError Parse(string? error)
        {
            if (string.IsNullOrWhiteSpace(error))
            {
                return new AdapterError(AdapterErrorCode.Error, "unknown");
            }

            var lower = error!.Trim().ToLowerInvariant();

            if (lower.Contains("no_fill") || lower.Contains("no fill"))
            {
                return new AdapterError(AdapterErrorCode.NoFill, error, recoverable: true);
            }

            if (lower.Contains("below_floor") || lower.Contains("below floor"))
            {
                return new AdapterError(AdapterErrorCode.BelowFloor, error, recoverable: true);
            }

            if (lower.Contains("timeout"))
            {
                return new AdapterError(AdapterErrorCode.Timeout, error, recoverable: true);
            }

            if (lower.Contains("network"))
            {
                return new AdapterError(AdapterErrorCode.NetworkError, error, recoverable: true);
            }

            if (TryParseStatus(lower, out var statusCode))
            {
                if (statusCode >= 400 && statusCode < 500)
                {
                    return new AdapterError(AdapterErrorCode.Status4xx, error, vendorCode: statusCode.ToString());
                }

                if (statusCode >= 500 && statusCode < 600)
                {
                    return new AdapterError(AdapterErrorCode.Status5xx, error, vendorCode: statusCode.ToString());
                }
            }

            if (lower.Contains("status_4xx") || lower.Contains("4xx"))
            {
                return new AdapterError(AdapterErrorCode.Status4xx, error);
            }

            if (lower.Contains("status_5xx") || lower.Contains("5xx"))
            {
                return new AdapterError(AdapterErrorCode.Status5xx, error);
            }

            if (lower.Contains("circuit_open") || lower.Contains("circuit"))
            {
                return new AdapterError(AdapterErrorCode.CircuitOpen, error, recoverable: true);
            }

            if (lower.Contains("config"))
            {
                return new AdapterError(AdapterErrorCode.Config, error);
            }

            return new AdapterError(AdapterErrorCode.Error, error);
        }

        private static bool TryParseStatus(string lower, out int statusCode)
        {
            statusCode = 0;
            const string prefix = "status_";
            var idx = lower.IndexOf(prefix, StringComparison.Ordinal);
            if (idx < 0 || idx + prefix.Length >= lower.Length)
            {
                return false;
            }

            var slice = lower[(idx + prefix.Length)..];
            if (int.TryParse(slice, out var parsed))
            {
                statusCode = parsed;
                return true;
            }

            return false;
        }
    }
}
