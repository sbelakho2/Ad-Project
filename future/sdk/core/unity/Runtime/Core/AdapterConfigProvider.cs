using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    /// <summary>
    /// Host app supplied provider for adapter-specific credentials.
    /// </summary>
    public interface IAdapterConfigProvider
    {
        IReadOnlyDictionary<string, object?>? GetConfig(string network);
    }

    /// <summary>
    /// Helper for null-pattern implementation.
    /// </summary>
    public sealed class NullAdapterConfigProvider : IAdapterConfigProvider
    {
        public static readonly NullAdapterConfigProvider Instance = new();

        private NullAdapterConfigProvider()
        {
        }

        public IReadOnlyDictionary<string, object?>? GetConfig(string network)
        {
            return null;
        }
    }
}
