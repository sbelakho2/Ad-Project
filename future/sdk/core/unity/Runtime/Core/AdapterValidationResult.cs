using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    public sealed class AdapterValidationResult
    {
        public AdapterValidationResult(string network, bool success, string message, IReadOnlyCollection<string>? missingKeys = null)
        {
            Network = network;
            Success = success;
            Message = message;
            MissingKeys = missingKeys ?? new List<string>();
        }

        public string Network { get; }
        public bool Success { get; }
        public string Message { get; }
        public IReadOnlyCollection<string> MissingKeys { get; }
    }
}
