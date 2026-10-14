# ApexMediation SDK Ecosystem

High-performance mediation SDKs for mobile, web, and CTV. Designed for minimal footprint, maximum reliability, and full transparency.

## 📱 Supported Platforms

- **Android**: Native Kotlin SDK with Gradle integration.
- **iOS**: Native Swift SDK with CocoaPods and Swift Package Manager support.
- **Unity**: C# wrapper for Android/iOS native cores with IL2CPP compatibility.
- **Web**: Lightweight JavaScript/TypeScript SDK for browser-based environments.
- **CTV**: Specialized SDKs for Apple TV (tvOS), Android TV, and Web-based CTV platforms.

## ✨ Key Features

- **Ultra-Lean**: Core SDKs are <500KB in size.
- **OTA-Proof**: Configurations are cryptographically signed (Ed25519) to prevent remote exploits or breakage.
- **Performance**: Optimized for <100ms cold start and <0.02% ANR contribution.
- **Privacy First**: Built-in support for GDPR (TCF v2), CCPA, and COPPA.
- **Transparency**: Per-impression bid landscapes available via local APIs.

## 🏗️ Architecture

The SDK ecosystem follows a "Core + Adapters" architecture:

1.  **Core SDK**: Handles initialization, configuration caching, telemetry, and auction orchestration.
2.  **Adapters**: Platform-specific bindings for third-party ad networks (AdMob, AppLovin, Meta, etc.).
3.  **Circuit Breaker**: Protects the app from misbehaving third-party SDKs by automatically disabling failing adapters.

## 🛠️ Development

### Prerequisites
- Android Studio / JDK 17
- Xcode 14+
- Unity 2021.3+
- Node.js 20+ (for Web SDK)

### Structure
```text
sdk/
├── core/               # Shared logic and native cores
├── adapter-dev-kit/    # Tools for building new network adapters
├── web/                # Web SDK implementation
└── ctv/                # CTV-specific platforms
```

## 🧪 Testing

We use a "Contract First" testing strategy:
- **Unit Tests**: Platform-specific unit tests for core logic.
- **Contract Tests**: Ensures parity between Android and iOS implementations.
- **Sandbox Apps**: Real test apps for each platform to verify E2E behavior.

Run the full system check to verify SDK parity:
```bash
./tools/ci/full_system_check.sh --steps sdk
```
