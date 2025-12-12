# ProGuard/R8 Obfuscation Rules for Ad Networks

## Overview

When using ProGuard (Android) or R8 (Android Gradle Plugin 3.4+) with ad SDKs, improper obfuscation can cause runtime crashes, broken callbacks, and ad loading failures. This document provides comprehensive keep rules for all major ad networks.

---

## Core Apex Mediation SDK Rules

```proguard
# ==============================================================================
# Apex Mediation SDK - Core Rules
# ==============================================================================

# Keep all public API
-keep class com.apexmediation.** { *; }
-keepclassmembers class com.apexmediation.** { *; }

# Keep callback interfaces
-keepclassmembers class * implements com.apexmediation.AdCallback {
    public *;
}

# Keep lifecycle handlers
-keepclassmembers class * implements com.apexmediation.LifecycleAware {
    public *;
}

# Preserve annotations
-keepattributes *Annotation*
-keepattributes Signature
-keepattributes InnerClasses
-keepattributes EnclosingMethod

# Keep native method names
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep Parcelables
-keepclassmembers class * implements android.os.Parcelable {
    public static final android.os.Parcelable$Creator CREATOR;
}

# Keep Serializable
-keepclassmembers class * implements java.io.Serializable {
    static final long serialVersionUID;
    private static final java.io.ObjectStreamField[] serialPersistentFields;
    !static !transient <fields>;
    private void writeObject(java.io.ObjectOutputStream);
    private void readObject(java.io.ObjectInputStream);
    java.lang.Object writeReplace();
    java.lang.Object readResolve();
}
```

---

## Google AdMob / Google Mobile Ads

```proguard
# ==============================================================================
# Google Mobile Ads SDK
# Version: 23.x
# ==============================================================================

-keep class com.google.android.gms.ads.** { *; }
-keep class com.google.ads.** { *; }

# Mediation adapters
-keep class com.google.ads.mediation.** { *; }
-keepclassmembers class com.google.ads.mediation.** { *; }

# Internal classes required for reflection
-keep class com.google.android.gms.internal.ads.** { *; }

# Open Measurement SDK (viewability)
-keep class com.iab.omid.** { *; }
-dontwarn com.iab.omid.**

# Keep WebView JavaScript interface
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# MRAID bridge
-keepclassmembers class com.google.android.gms.ads.internal.webview.** {
    public *;
}

# Preserve ad format classes
-keep class com.google.android.gms.ads.AdView { *; }
-keep class com.google.android.gms.ads.InterstitialAd { *; }
-keep class com.google.android.gms.ads.rewarded.RewardedAd { *; }
-keep class com.google.android.gms.ads.nativead.NativeAd { *; }
-keep class com.google.android.gms.ads.appopen.AppOpenAd { *; }

# UMP (User Messaging Platform) for consent
-keep class com.google.android.ump.** { *; }
```

---

## Meta Audience Network (Facebook)

```proguard
# ==============================================================================
# Meta Audience Network
# Version: 6.x
# ==============================================================================

-keep class com.facebook.ads.** { *; }
-keepclassmembers class com.facebook.ads.** { *; }

# Internal bidding
-keep class com.facebook.ads.internal.** { *; }

# Native ad components
-keep class com.facebook.ads.NativeAd { *; }
-keep class com.facebook.ads.NativeAdLayout { *; }
-keep class com.facebook.ads.NativeBannerAd { *; }
-keep class com.facebook.ads.MediaView { *; }
-keep class com.facebook.ads.AdOptionsView { *; }

# Interstitial and rewarded
-keep class com.facebook.ads.InterstitialAd { *; }
-keep class com.facebook.ads.RewardedVideoAd { *; }

# Bidding
-keep class com.facebook.bidding.** { *; }

# ExoPlayer (video ads)
-keep class com.google.android.exoplayer2.** { *; }
-dontwarn com.google.android.exoplayer2.**
```

---

## AppLovin MAX

```proguard
# ==============================================================================
# AppLovin MAX SDK
# Version: 12.x
# ==============================================================================

-keep class com.applovin.** { *; }
-keepclassmembers class com.applovin.** { *; }

# SDK core
-keep class com.applovin.sdk.AppLovinSdk { *; }
-keep class com.applovin.sdk.AppLovinSdkConfiguration { *; }

# Ad formats
-keep class com.applovin.mediation.** { *; }
-keep class com.applovin.mediation.ads.** { *; }
-keep class com.applovin.mediation.adapters.** { *; }

# MAX ad loader
-keep class com.applovin.mediation.MaxAd { *; }
-keep class com.applovin.mediation.MaxAdListener { *; }
-keep class com.applovin.mediation.MaxRewardedAdListener { *; }

# Native ads
-keep class com.applovin.mediation.nativeAds.** { *; }

# Reflection-based initialization
-keepclassmembers class com.applovin.sdk.AppLovinSdk {
    public static *** getInstance(...);
}

# Privacy and consent
-keep class com.applovin.sdk.AppLovinPrivacySettings { *; }
```

---

## Unity Ads

```proguard
# ==============================================================================
# Unity Ads SDK
# Version: 4.x
# ==============================================================================

-keep class com.unity3d.ads.** { *; }
-keepclassmembers class com.unity3d.ads.** { *; }

# Core classes
-keep class com.unity3d.ads.UnityAds { *; }
-keep class com.unity3d.ads.IUnityAdsListener { *; }
-keep class com.unity3d.ads.IUnityAdsLoadListener { *; }
-keep class com.unity3d.ads.IUnityAdsShowListener { *; }

# Services
-keep class com.unity3d.services.** { *; }
-keepclassmembers class com.unity3d.services.** { *; }

# Monetization
-keep class com.unity3d.services.monetization.** { *; }

# Banner ads
-keep class com.unity3d.services.banners.** { *; }

# WebView bridge
-keep class com.unity3d.services.ads.webplayer.** { *; }
```

---

## ironSource

```proguard
# ==============================================================================
# ironSource SDK
# Version: 8.x
# ==============================================================================

-keep class com.ironsource.** { *; }
-keepclassmembers class com.ironsource.** { *; }

# Core SDK
-keep class com.ironsource.mediationsdk.** { *; }
-keep class com.ironsource.sdk.** { *; }

# Adapters
-keep class com.ironsource.adapters.** { *; }

# Listeners
-keep interface com.ironsource.mediationsdk.IronSource$IInitializationListener { *; }
-keep interface com.ironsource.mediationsdk.sdk.* { *; }

# SuperSonic (legacy)
-keep class com.supersonicads.** { *; }
-dontwarn com.supersonicads.**
```

---

## Vungle / Liftoff Monetize

```proguard
# ==============================================================================
# Vungle SDK (Liftoff Monetize)
# Version: 7.x
# ==============================================================================

-keep class com.vungle.ads.** { *; }
-keepclassmembers class com.vungle.ads.** { *; }

# Core
-keep class com.vungle.ads.VungleAds { *; }
-keep class com.vungle.ads.VungleInitializer { *; }

# Ad formats
-keep class com.vungle.ads.InterstitialAd { *; }
-keep class com.vungle.ads.RewardedAd { *; }
-keep class com.vungle.ads.BannerAd { *; }
-keep class com.vungle.ads.NativeAd { *; }

# Listeners
-keep interface com.vungle.ads.VungleInitializer.InitializationListener { *; }
-keep interface com.vungle.ads.BaseAdListener { *; }

# Internal
-keep class com.vungle.ads.internal.** { *; }
-dontwarn com.vungle.ads.internal.**

# OkHttp (networking)
-dontwarn okhttp3.**
-dontwarn okio.**
-keep class okhttp3.** { *; }
-keep class okio.** { *; }
```

---

## Mintegral

```proguard
# ==============================================================================
# Mintegral SDK
# Version: 16.x
# ==============================================================================

-keep class com.mbridge.** { *; }
-keepclassmembers class com.mbridge.** { *; }

# Core
-keep class com.mbridge.msdk.** { *; }
-keep class com.mbridge.msdk.foundation.** { *; }

# Ad formats
-keep class com.mbridge.msdk.out.** { *; }
-keep class com.mbridge.msdk.interstitialvideo.** { *; }
-keep class com.mbridge.msdk.reward.** { *; }
-keep class com.mbridge.msdk.banner.** { *; }
-keep class com.mbridge.msdk.nativex.** { *; }
```

---

## Moloco

```proguard
# ==============================================================================
# Moloco SDK
# Version: 3.x
# Note: Moloco uses server-to-server (S2S) integration primarily.
# These rules are for the optional client-side SDK components.
# ==============================================================================

-keep class com.moloco.** { *; }
-keepclassmembers class com.moloco.** { *; }

# Core
-keep class com.moloco.sdk.MolocoSdk { *; }
-keep class com.moloco.sdk.MolocoAdRequest { *; }

# Listeners
-keep interface com.moloco.sdk.listeners.** { *; }

# Internal
-dontwarn com.moloco.**
```

---

## Chartboost

```proguard
# ==============================================================================
# Chartboost SDK
# Version: 9.x
# ==============================================================================

-keep class com.chartboost.** { *; }
-keepclassmembers class com.chartboost.** { *; }

# SDK core
-keep class com.chartboost.sdk.Chartboost { *; }
-keep class com.chartboost.sdk.ChartboostDelegate { *; }

# Ads
-keep class com.chartboost.sdk.ads.** { *; }
-keep class com.chartboost.sdk.callbacks.** { *; }

# Privacy
-keep class com.chartboost.sdk.privacy.** { *; }
```

---

## AdColony (Digital Turbine)

```proguard
# ==============================================================================
# AdColony SDK (Digital Turbine)
# Version: 4.x
# ==============================================================================

-keep class com.adcolony.** { *; }
-keepclassmembers class com.adcolony.** { *; }

# Core
-keep class com.adcolony.sdk.AdColony { *; }
-keep class com.adcolony.sdk.AdColonyInterstitial { *; }
-keep class com.adcolony.sdk.AdColonyRewardListener { *; }

# Banner
-keep class com.adcolony.sdk.AdColonyAdView { *; }

# Internal
-dontwarn com.adcolony.sdk.**
```

---

## Pangle (TikTok/ByteDance)

```proguard
# ==============================================================================
# Pangle SDK
# Version: 5.x
# ==============================================================================

-keep class com.bytedance.sdk.** { *; }
-keepclassmembers class com.bytedance.sdk.** { *; }

-keep class com.pangle.sdk.** { *; }
-keepclassmembers class com.pangle.sdk.** { *; }

# Core
-keep class com.bytedance.sdk.openadsdk.** { *; }

# Ad formats
-keep class com.bytedance.sdk.openadsdk.TTAdNative { *; }
-keep class com.bytedance.sdk.openadsdk.TTNativeAd { *; }
-keep class com.bytedance.sdk.openadsdk.TTRewardVideoAd { *; }
-keep class com.bytedance.sdk.openadsdk.TTFullScreenVideoAd { *; }

# Multidex
-keep class com.bytedance.** { *; }
-dontwarn com.bytedance.**
```

---

## Amazon Publisher Services

```proguard
# ==============================================================================
# Amazon Publisher Services
# Version: 9.x
# ==============================================================================

-keep class com.amazon.device.ads.** { *; }
-keepclassmembers class com.amazon.device.ads.** { *; }

# DTB (Transparent Ad Marketplace)
-keep class com.amazon.device.ads.DTBAdRequest { *; }
-keep class com.amazon.device.ads.DTBAdResponse { *; }
-keep class com.amazon.device.ads.DTBAdCallback { *; }

# Modular ads
-keep class com.amazon.device.ads.modular.** { *; }
```

---

## Fyber / DT Exchange

```proguard
# ==============================================================================
# Fyber / Digital Turbine Exchange
# Version: 8.x
# ==============================================================================

-keep class com.fyber.** { *; }
-keepclassmembers class com.fyber.** { *; }

-keep class com.fyber.inneractive.sdk.** { *; }

# Marketplace
-keep class com.fyber.fairbid.** { *; }

# Internal
-dontwarn com.fyber.**
```

---

## Smaato

```proguard
# ==============================================================================
# Smaato SDK
# Version: 22.x
# ==============================================================================

-keep class com.smaato.sdk.** { *; }
-keepclassmembers class com.smaato.sdk.** { *; }

# Core
-keep class com.smaato.sdk.core.SmaatoSdk { *; }

# Banner
-keep class com.smaato.sdk.banner.** { *; }

# Interstitial
-keep class com.smaato.sdk.interstitial.** { *; }

# Rewarded
-keep class com.smaato.sdk.rewarded.** { *; }

# Native
-keep class com.smaato.sdk.nativead.** { *; }
```

---

## Common Libraries Used by Ad SDKs

```proguard
# ==============================================================================
# Common Dependencies
# ==============================================================================

# OkHttp
-dontwarn okhttp3.**
-dontwarn okio.**
-keep class okhttp3.** { *; }
-keep interface okhttp3.** { *; }

# Retrofit
-dontwarn retrofit2.**
-keep class retrofit2.** { *; }
-keepattributes Signature
-keepattributes Exceptions

# Gson
-keep class com.google.gson.** { *; }
-keepattributes Signature
-keepattributes *Annotation*
-keep class * implements com.google.gson.TypeAdapterFactory
-keep class * implements com.google.gson.JsonSerializer
-keep class * implements com.google.gson.JsonDeserializer

# Protobuf
-keep class com.google.protobuf.** { *; }
-dontwarn com.google.protobuf.**

# ExoPlayer (video)
-keep class com.google.android.exoplayer2.** { *; }
-dontwarn com.google.android.exoplayer2.**

# Glide (image loading)
-keep public class * implements com.bumptech.glide.module.GlideModule
-keep class * extends com.bumptech.glide.module.AppGlideModule {
    <init>(...);
}
-keep public enum com.bumptech.glide.load.ImageHeaderParser$** {
    **[] $VALUES;
    public *;
}
-keep class com.bumptech.glide.load.data.ParcelFileDescriptorRewinder$InternalRewinder {
    *** rewind();
}

# AndroidX
-keep class androidx.** { *; }
-keep interface androidx.** { *; }
-dontwarn androidx.**

# Google Play Services
-keep class com.google.android.gms.** { *; }
-dontwarn com.google.android.gms.**
```

---

## Complete proguard-rules.pro Template

```proguard
# ==============================================================================
# COMPLETE AD SDK PROGUARD RULES
# 
# Add this file to your app's proguard-rules.pro or as a separate file
# referenced in build.gradle:
#
# android {
#     buildTypes {
#         release {
#             proguardFiles getDefaultProguardFile('proguard-android-optimize.txt'),
#                          'proguard-rules.pro',
#                          'proguard-ads.pro'
#         }
#     }
# }
# ==============================================================================

# General Android
-keepattributes *Annotation*
-keepattributes Signature
-keepattributes SourceFile,LineNumberTable
-keepattributes InnerClasses,EnclosingMethod

# JavaScript interfaces (WebView ads)
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# Parcelables
-keepclassmembers class * implements android.os.Parcelable {
    public static final ** CREATOR;
}

# R8 full mode compatibility
-keep,allowobfuscation,allowshrinking class kotlin.coroutines.Continuation
-keep,allowobfuscation,allowshrinking interface retrofit2.Call
-keep,allowobfuscation,allowshrinking class retrofit2.Response

# ==============================================================================
# Include all ad network rules from above sections
# ==============================================================================

# --- Apex Mediation ---
-keep class com.apexmediation.** { *; }

# --- Google AdMob ---
-keep class com.google.android.gms.ads.** { *; }
-keep class com.google.ads.** { *; }

# --- Meta Audience Network ---
-keep class com.facebook.ads.** { *; }

# --- AppLovin MAX ---
-keep class com.applovin.** { *; }

# --- Unity Ads ---
-keep class com.unity3d.ads.** { *; }
-keep class com.unity3d.services.** { *; }

# --- ironSource ---
-keep class com.ironsource.** { *; }

# --- Vungle ---
-keep class com.vungle.ads.** { *; }

# --- Mintegral ---
-keep class com.mbridge.** { *; }

# --- Moloco ---
-keep class com.moloco.** { *; }

# --- Chartboost ---
-keep class com.chartboost.** { *; }

# --- AdColony ---
-keep class com.adcolony.** { *; }

# --- Pangle ---
-keep class com.bytedance.sdk.** { *; }
-keep class com.pangle.sdk.** { *; }

# --- Amazon ---
-keep class com.amazon.device.ads.** { *; }

# --- Fyber ---
-keep class com.fyber.** { *; }

# --- Smaato ---
-keep class com.smaato.sdk.** { *; }

# --- Common libraries ---
-keep class okhttp3.** { *; }
-keep class okio.** { *; }
-keep class com.google.gson.** { *; }
-keep class retrofit2.** { *; }
-keep class com.google.protobuf.** { *; }
-keep class com.google.android.exoplayer2.** { *; }

# Suppress warnings for optional dependencies
-dontwarn com.google.**
-dontwarn com.facebook.**
-dontwarn com.applovin.**
-dontwarn com.unity3d.**
-dontwarn com.ironsource.**
-dontwarn com.vungle.**
-dontwarn com.mbridge.**
-dontwarn com.moloco.**
-dontwarn com.chartboost.**
-dontwarn com.adcolony.**
-dontwarn com.bytedance.**
-dontwarn com.amazon.**
-dontwarn com.fyber.**
-dontwarn com.smaato.**
```

---

## Debugging Obfuscation Issues

### Enable Mapping File

```groovy
// build.gradle
android {
    buildTypes {
        release {
            minifyEnabled true
            proguardFiles getDefaultProguardFile('proguard-android-optimize.txt'),
                         'proguard-rules.pro'
            
            // Generate mapping for crash reports
            proguardFiles.each { file ->
                println "ProGuard: ${file.name}"
            }
        }
    }
}
```

### Decode Stack Traces

```bash
# Decode obfuscated crash
retrace mapping.txt stacktrace.txt

# Or use Android Studio:
# Build > Analyze APK > Select mapping.txt
```

### Verify Keep Rules

```bash
# Print kept classes
-printseeds seeds.txt

# Print removed classes  
-printusage usage.txt

# Print class mappings
-printmapping mapping.txt
```

---

## Version-Specific Notes

| SDK | Version | Special Notes |
|-----|---------|--------------|
| AdMob | 23.x | Requires OMSDK rules |
| AppLovin | 12.x | Native ad rules added |
| Unity Ads | 4.x | Services package separate |
| ironSource | 8.x | LevelPlay integration |
| Vungle | 7.x | Rebranded to Liftoff |
| Pangle | 5.x | ByteDance namespace |

---

## Testing Obfuscated Builds

1. **Build release APK**
2. **Test all ad formats** (banner, interstitial, rewarded, native)
3. **Verify callbacks fire** (onAdLoaded, onAdFailed, etc.)
4. **Check logging** for "class not found" errors
5. **Monitor crash reports** for obfuscation issues

For support, contact sdk-support@apexmediation.ee with:
- ProGuard/R8 version
- Full mapping.txt file
- Stack trace of any crashes
