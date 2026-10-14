namespace Apex.Mediation.Adapters
{
    /// <summary>
    /// Marker interface describing vendor SDK adapters.
    /// </summary>
    public interface IAdapter
    {
        string Network { get; }
        bool SupportsS2S { get; }
    }
}
