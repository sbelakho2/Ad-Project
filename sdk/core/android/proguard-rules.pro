# Rival Apex Mediation SDK consumer rules.
#
# Host apps reach the SDK through MediationSDK and the Bel* facades, and BYO
# adapters implement the adapter contracts; keep those public surfaces so
# shrinking never strips entry points. Internal implementation classes stay
# shrinkable.
-keep public class com.rivalapexmediation.sdk.MediationSDK { public *; }
-keep public class com.rivalapexmediation.sdk.BelAds { public *; }
-keep public class com.rivalapexmediation.sdk.BelBanner { public *; }
-keep public class com.rivalapexmediation.sdk.BelInterstitial { public *; }
-keep public class com.rivalapexmediation.sdk.BelRewarded { public *; }
-keep public class com.rivalapexmediation.sdk.BelRewardedInterstitial { public *; }
-keep public class com.rivalapexmediation.sdk.BelAppOpen { public *; }
-keep public interface com.rivalapexmediation.sdk.adapter.AdNetworkAdapter { *; }
-keep public interface com.rivalapexmediation.sdk.contract.AdNetworkAdapterV2 { *; }
