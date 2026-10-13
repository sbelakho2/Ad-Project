using Apex.Mediation.Core;
using NUnit.Framework;

namespace Apex.Mediation.Tests
{
    public class Il2CppCompatibilityTests
    {
        [Test]
        public void PublicApi_survives_linking()
        {
            // Ensure the assembly loads and key types are preserved for AOT/IL2CPP.
            // (NUnit 4 API: classic Assert.NotNull was removed in favor of constraint asserts.)
            Assert.That(typeof(MediationSDK), Is.Not.Null);
            Assert.That(typeof(ApexConfig), Is.Not.Null);
        }
    }
}
