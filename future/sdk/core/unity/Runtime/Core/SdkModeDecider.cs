using System.Collections.Generic;

namespace Apex.Mediation.Core
{
    internal static class SdkModeDecider
    {
        public static bool ShouldUseS2S(SdkMode mode, IEnumerable<NetworkAdapterDescriptor> adapters, CredentialStore credentialStore)
        {
            if (mode != SdkMode.BYO)
            {
                return true;
            }

            return credentialStore != null && credentialStore.HasS2SCreds(adapters);
        }
    }
}
