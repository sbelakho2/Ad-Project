# TV SDK Integration (Android TV & tvOS)

## Overview

The Apex Mediation SDK supports Connected TV (CTV) platforms, specifically Android TV (and Fire TV) and Apple tvOS. The integration is very similar to the mobile SDKs but requires special attention to user interface and navigation (D-Pad/Remote).

## Android TV / Fire TV

### 1. Dependencies

Add the CTV module to your `build.gradle`.

```groovy
implementation 'com.rivalapexmediation.ctv:android-tv:0.1.0'
```

### 2. UI & Navigation

When showing ads on TV, ensure your app handles focus correctly.

*   **Interstitial Ads**: The SDK handles focus automatically.
*   **Native/Banner Ads**: You must ensure the ad view is focusable if it's interactive.

```xml
<com.apexmediation.sdk.BannerView
    android:id="@+id/banner_ad"
    android:layout_width="wrap_content"
    android:layout_height="wrap_content"
    android:focusable="true"
    android:focusableInTouchMode="true" />
```

### 3. Manifest

Declare touchscreen as not required in `AndroidManifest.xml`:

```xml
<uses-feature android:name="android.hardware.touchscreen" android:required="false" />
<uses-feature android:name="android.software.leanback" android:required="true" />
```

## Apple tvOS

### 1. Dependencies

Add the tvOS specific package via Swift Package Manager.

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/sbelakho2/apex-mediation-platform", from: "1.0.0")
]
```

Or via CocoaPods:

```ruby
platform :tvos, '14.0'
target 'MyAppTV' do
  pod 'CTVSDK', '~> 1.0.0'
end
```

### 2. Focus Engine

For native ads, ensure your views override `canBecomeFocused`.

```swift
class AdView: UIView {
    override var canBecomeFocused: Bool {
        return true
    }
    
    override func didUpdateFocus(in context: UIFocusUpdateContext, with coordinator: UIFocusAnimationCoordinator) {
        // Handle focus visual changes (scale, shadow)
    }
}
```

## Key Concepts

*   **Video First**: CTV inventory is predominantly video. Ensure you are requesting video-enabled placements.
*   **Vast Tags**: Many CTV integrations rely on VAST tags. The SDK handles parsing and rendering, but ensure your network setup supports VAST 4.x.

## Integration Checklist

1.  [ ] (Android) Added `leanback` feature to Manifest.
2.  [ ] (iOS) Targeted `tvos` platform in Podfile.
3.  [ ] Verified navigation works with a physical remote or D-Pad simulator.
4.  [ ] Tested video ad playback and completion tracking.
