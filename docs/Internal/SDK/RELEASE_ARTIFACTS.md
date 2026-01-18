# SDK Release Artifacts & Distribution

**Last Updated:** January 2026  
**Owner:** SDK Team

---

## Overview

This document describes the release artifacts and distribution channels for Apex Mediation SDKs across all supported platforms.

---

## 1. Android SDK

### 1.1 Maven Repository

| Artifact | Group ID | Artifact ID | Current Version |
|----------|----------|-------------|-----------------|
| Core SDK | `com.rivalapexmediation.sdk` | `core` | 1.0.0 |
| AdMob Adapter | `com.rivalapexmediation.sdk` | `adapter-admob` | 1.0.0 |
| AppLovin Adapter | `com.rivalapexmediation.sdk` | `adapter-applovin` | 1.0.0 |
| Unity Adapter | `com.rivalapexmediation.sdk` | `adapter-unity` | 1.0.0 |
| IronSource Adapter | `com.rivalapexmediation.sdk` | `adapter-ironsource` | 1.0.0 |
| Meta Adapter | `com.rivalapexmediation.sdk` | `adapter-meta` | 1.0.0 |
| Vungle Adapter | `com.rivalapexmediation.sdk` | `adapter-vungle` | 1.0.0 |
| Chartboost Adapter | `com.rivalapexmediation.sdk` | `adapter-chartboost` | 1.0.0 |
| Pangle Adapter | `com.rivalapexmediation.sdk` | `adapter-pangle` | 1.0.0 |
| Mintegral Adapter | `com.rivalapexmediation.sdk` | `adapter-mintegral` | 1.0.0 |
| AdColony Adapter | `com.rivalapexmediation.sdk` | `adapter-adcolony` | 1.0.0 |
| Tapjoy Adapter | `com.rivalapexmediation.sdk` | `adapter-tapjoy` | 1.0.0 |
| Moloco Adapter | `com.rivalapexmediation.sdk` | `adapter-moloco` | 1.0.0 |
| Fyber Adapter | `com.rivalapexmediation.sdk` | `adapter-fyber` | 1.0.0 |
| Smaato Adapter | `com.rivalapexmediation.sdk` | `adapter-smaato` | 1.0.0 |
| Amazon Adapter | `com.rivalapexmediation.sdk` | `adapter-amazon` | 1.0.0 |
| CTV (Android TV) | `com.rivalapexmediation.ctv` | `android-tv` | 0.1.0 |

### 1.2 Repository URL

```groovy
// build.gradle (project level)
repositories {
    mavenCentral()
    maven { url "https://repo.apexmediation.com/android" }
}
```

### 1.3 Release Process

1. **CI/CD:** GitHub Actions builds and tests on every push
2. **Signing:** AAR signed with release keystore
3. **Publication:** Automatic publish to Maven repo on release tag
4. **Changelog:** Auto-generated from conventional commits

---

## 2. iOS SDK

### 2.1 CocoaPods

| Pod Name | Current Version | Subspecs |
|----------|-----------------|----------|
| `ApexMediation` | 1.0.0 | Core, AdMob, AppLovin, Unity, etc. |

**Podfile example:**
```ruby
pod 'ApexMediation', '~> 1.0.0'
pod 'ApexMediation/AdMob'
pod 'ApexMediation/AppLovin'
```

**CocoaPods Trunk Status:** Published ✅

### 2.2 Swift Package Manager

**Repository URL:**
```
https://github.com/sbelakho2/apex-mediation-platform
```

**Package.swift products:**
- `ApexMediation` (core SDK)
- `ApexMediationAdMob`
- `ApexMediationAppLovin`
- (other adapter products)

### 2.3 Release Process

1. **CI/CD:** Xcode Cloud / GitHub Actions
2. **Signing:** Signed with Apple Developer certificate
3. **CocoaPods:** `pod trunk push` on release tag
4. **SPM:** Git tag triggers automatic availability

---

## 3. Unity SDK

### 3.1 Unity Package

| Artifact | Format | Current Version |
|----------|--------|-----------------|
| ApexMediation Unity SDK | `.unitypackage` | 1.0.0 |

### 3.2 Download Location

**GitHub Releases:**
```
https://github.com/sbelakho2/apex-mediation-platform/releases
```

Direct link:
```
https://github.com/sbelakho2/apex-mediation-platform/releases/download/v1.0.0/ApexMediation-1.0.0.unitypackage
```

### 3.3 Package Contents

```
ApexMediation.unitypackage
├── Editor/
│   ├── ApexMediationSettings.cs
│   ├── ExternalDependencyManager/
│   └── BuildPostProcessor.cs
├── Runtime/
│   ├── ApexMediation.cs
│   ├── Platforms/
│   └── Core/
├── Dependencies.xml (EDM4U)
└── package.json
```

### 3.4 Release Process

1. **Build:** `Assets > Export Package`
2. **Test:** Verify in Unity 2021.3+, 2022.x, 2023.x
3. **Upload:** Attach to GitHub Release
4. **Changelog:** Include in release notes

---

## 4. Web SDK

### 4.1 NPM Package

| Package | Version | Registry |
|---------|---------|----------|
| `@apexmediation/web-sdk` | 0.1.0 | npm |

**Installation:**
```bash
npm install @apexmediation/web-sdk
```

### 4.2 CDN

**jsDelivr:**
```html
<script src="https://cdn.jsdelivr.net/npm/@apexmediation/web-sdk@0.1.0/dist/index.umd.js"></script>
```

**Self-hosted:**
```html
<script src="https://cdn.apexmediation.com/sdk/web/v1/apex.min.js"></script>
```

### 4.3 Build Outputs

| Output | Path | Format |
|--------|------|--------|
| ESM | `dist/index.esm.js` | ES Modules |
| UMD | `dist/index.umd.js` | Universal |
| Types | `dist/src/index.d.ts` | TypeScript |

### 4.4 Release Process

1. **Build:** `npm run build`
2. **Test:** `npm test`
3. **Publish:** `npm publish`
4. **CDN:** Auto-synced via jsDelivr

---

## 5. CTV SDKs

### 5.1 Android TV

Distributed via same Maven repository as Android SDK:

```groovy
implementation 'com.rivalapexmediation.ctv:android-tv:0.1.0'
```

### 5.2 tvOS

Included in iOS SPM package:

```swift
.product(name: "ApexMediationCTV", package: "apex-mediation-platform")
```

---

## 6. Version Compatibility Matrix

### Android

| SDK Version | Min Android | Target Android | Kotlin | Java |
|-------------|-------------|----------------|--------|------|
| 1.0.0 | API 21 | API 34 | 1.9.0 | 11 |

### iOS

| SDK Version | Min iOS | Swift | Xcode |
|-------------|---------|-------|-------|
| 1.0.0 | 12.0 | 5.9+ | 15.0+ |

### Unity

| SDK Version | Min Unity | EDM4U Required |
|-------------|-----------|----------------|
| 1.0.0 | 2021.3 | Yes |

### Web

| SDK Version | Browsers |
|-------------|----------|
| 0.1.0 | Chrome 90+, Safari 14+, Firefox 90+, Edge 90+ |

---

## 7. Artifact Verification

### Android (SHA-256)

```bash
# Verify artifact signature
shasum -a 256 apexmediation-core-1.0.0.aar
# Expected: [checksum published in release notes]
```

### iOS (Code Signing)

All iOS frameworks are signed with:
- **Team ID:** [Your Team ID]
- **Certificate:** Apple Distribution

### Unity

Package includes `manifest.json` with integrity hashes.

---

## 8. Support Matrix

| Platform | Production Ready | Support Level |
|----------|------------------|---------------|
| Android | ✅ Yes | Full |
| iOS | ✅ Yes | Full |
| Unity (Android) | ✅ Yes | Full |
| Unity (iOS) | ✅ Yes | Full |
| Web | ✅ Yes | Full |
| Android TV | ✅ Yes | Full |
| tvOS | ✅ Yes | Full |

---

## 9. Update Notifications

To receive SDK update notifications:

1. **GitHub:** Watch releases at `github.com/sbelakho2/apex-mediation-platform`
2. **Console:** SDK version warnings shown in dashboard
3. **Email:** Configure in Console Settings > Notifications

---

## 10. Troubleshooting

### Android: Cannot resolve artifact

```groovy
// Ensure repository is added
repositories {
    maven { url "https://repo.apexmediation.com/android" }
}
```

### iOS: Pod not found

```bash
pod repo update
pod install
```

### Unity: Missing dependencies

1. Open **Assets > External Dependency Manager > Android Resolver > Force Resolve**
2. Ensure EDM4U is installed

### Web: CDN not loading

Check browser console for CORS or network errors. Use local bundle as fallback.
