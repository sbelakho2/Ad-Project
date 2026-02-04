# Adapter Samples (Reference Only)

> ⚠️ **DO NOT USE IN PRODUCTION** ⚠️
>
> These are **sample/reference implementations** only. They are intended to demonstrate
> how to implement the `AdNetworkAdapterV2` interface for various ad networks.

## BYO-First Architecture

ApexMediation follows a **Bring Your Own (BYO)** adapter model:

1. **Core SDK ships with NO vendor adapters** - Release builds have an empty adapter registry
2. **Publishers implement their own adapters** - Using the provided interface contracts
3. **Publishers bring their own vendor SDK dependencies** - Not bundled in ApexMediation

## Purpose of These Samples

These sample adapters are provided to:

- Demonstrate correct implementation of the `AdNetworkAdapterV2` interface
- Show proper error mapping and callback handling patterns
- Illustrate privacy consent propagation to vendor SDKs
- Provide a starting point for publishers building their own adapters

## Using These Samples

1. **Copy** the relevant adapter to your own project/module
2. **Add the vendor SDK dependency** (e.g., Meta Audience Network, Unity Ads, AppLovin MAX)
3. **Modify** as needed for your specific integration requirements
4. **Register** your adapter at runtime using `MediationSDK.registerAdapter(...)`

## Compile Guards

All sample adapters are gated behind compile-time flags:

### Android (Kotlin)
```kotlin
// Only compiles when BuildConfig.DEBUG is true OR apex.sandbox.adapters=1
#if BuildConfig.DEBUG || System.getProperty("apex.sandbox.adapters") == "1"
```

### iOS (Swift)
```swift
// Only compiles when DEBUG or APEX_SANDBOX_ADAPTERS is defined
#if DEBUG || APEX_SANDBOX_ADAPTERS
```

## Available Samples

| Network | Android | iOS |
|---------|---------|-----|
| Meta Audience Network | ✅ | ✅ |
| Unity Ads | ✅ | ❌ |
| AppLovin MAX | ✅ | ❌ |

## Official Documentation

For complete adapter development guidance:

- iOS/tvOS: `docs/Developer-Facing/DevKits/ios-tvos.md`
- Android/Android TV: `docs/Developer-Facing/DevKits/android-androidtv.md`
- Adapter Dev Kit Guide: `docs/Internal/Archived_Customer_Docs/AdapterDevKit.md`

## Runtime Registration Example

### iOS
```swift
// In your app, before initialize()
Task {
    await MediationSDK.shared.registerAdapter(
        networkName: "meta",
        adapterType: YourMetaAdapter.self
    )
}
```

### Android
```kotlin
// In your Application.onCreate(), before initialize()
MediationSDK.registerRuntimeAdapterFactory("meta") { context ->
    YourMetaAdapter(context)
}
```
