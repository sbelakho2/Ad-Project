# Consumer ProGuard (R8) rules for apps integrating the Rival Apex Mediation Android SDK
# These rules are packaged with the AAR and applied in the app using the SDK.

# Keep the public SDK API surface so obfuscation doesn't break integrators
-keep class com.rivalapexmediation.sdk.** { *; }
-keep class com.rivalapexmediation.sdk.models.** { *; }
-keep interface com.rivalapexmediation.sdk.** { *; }

# Keep adapter contracts and callback interfaces used via reflection
-keep interface com.rivalapexmediation.sdk.adapters.** { *; }
-keep class com.rivalapexmediation.sdk.adapters.** { *; }

# Keep parcelables and their CREATOR fields for interop
-keepclassmembers class * implements android.os.Parcelable {
	public static final android.os.Parcelable$Creator *;
}

# Keep Kotlin metadata (for reflection and default args)
-keep class kotlin.Metadata { *; }

# Gson model classes are accessed reflectively
-keep class com.rivalapexmediation.sdk.models.** { *; }
-keepclassmembers class com.rivalapexmediation.sdk.models.** { <fields>; <methods>; }

# Keep enums and sealed error reasons used in callbacks/telemetry
-keepclassmembers enum * { **[] $VALUES; ** valueOf(java.lang.String); }

# OkHttp/Retrofit and Gson types that may be reflectively accessed
-dontwarn okio.**
-dontwarn javax.annotation.**
-dontwarn org.codehaus.mojo.animal_sniffer.*

# Coroutines debug metadata
-keep class kotlinx.coroutines.internal.MainDispatcherLoader { *; }
-keep class kotlinx.coroutines.android.AndroidDispatcherFactory { *; }

# Keep annotations used by the SDK
-keepattributes *Annotation*

# Keep enums and their values for stable error codes
-keepclassmembers enum * { **[] $VALUES; ** valueOf(java.lang.String); }
