namespace Apex.Mediation.Core
{
    /// <summary>
    /// Represents how the SDK should operate with respect to hosted demand vs. publisher-owned adapters.
    /// </summary>
    public enum SdkMode
    {
        BYO = 0,
        HYBRID = 1,
        MANAGED = 2
    }
}
