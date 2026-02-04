# AdProject Flutter SDK

Official Flutter plugin for the AdProject Mediation SDK - cross-platform ad monetization for iOS and Android.

## Features

- 🎯 **Multi-format ads**: Banner, Interstitial, Rewarded, Rewarded Interstitial, App Open
- 📊 **S2S Header Bidding**: Signal collection for server-to-server integration
- 🔒 **Privacy Compliance**: GDPR, CCPA, COPPA support
- 📈 **Health Monitoring**: Real-time adapter status and metrics
- 🎨 **Adaptive Banners**: Screen-width optimized banners

## Installation

Add to your `pubspec.yaml`:

```yaml
dependencies:
  adproject_sdk: ^1.0.0
```

Then run:

```bash
flutter pub get
```

### iOS Setup

Add to your `ios/Podfile`:

```ruby
pod 'AdProjectSDK', '~> 1.0'
```

### Android Setup

Add to your `android/build.gradle`:

```gradle
allprojects {
    repositories {
        maven { url 'https://maven.adproject.io/releases' }
    }
}
```

## Quick Start

### Initialize the SDK

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  
  final status = await AdProjectSdk.instance.initialize(
    'YOUR_APP_ID',
    testMode: kDebugMode,
    verbose: kDebugMode,
  );
  
  print('Initialized: ${status.isInitialized}');
  print('Adapters: ${status.adapters.length}');
  
  runApp(MyApp());
}
```

### Banner Ads

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

class MyScreen extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        // Standard banner
        BannerAd(
          adUnitId: 'YOUR_BANNER_AD_UNIT_ID',
          size: AdSize.banner,
          onAdLoaded: (result) => print('Banner loaded: ${result.networkName}'),
          onAdFailedToLoad: (error) => print('Banner failed: ${error.message}'),
          onAdImpression: (data) => print('Revenue: ${data.revenue}'),
        ),
        
        // Adaptive banner
        BannerAd(
          adUnitId: 'YOUR_BANNER_AD_UNIT_ID',
          size: AdSize.adaptive(MediaQuery.of(context).size.width.toInt()),
          autoRefresh: true,
          refreshInterval: Duration(seconds: 60),
        ),
      ],
    );
  }
}
```

### Interstitial Ads

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

class MyScreen extends StatefulWidget {
  @override
  State<MyScreen> createState() => _MyScreenState();
}

class _MyScreenState extends State<MyScreen> {
  late InterstitialAd _interstitialAd;

  @override
  void initState() {
    super.initState();
    _interstitialAd = InterstitialAd(adUnitId: 'YOUR_INTERSTITIAL_AD_UNIT_ID');
    _loadInterstitial();
  }

  Future<void> _loadInterstitial() async {
    final result = await _interstitialAd.load();
    if (result.success) {
      print('Interstitial loaded');
    }
  }

  Future<void> _showInterstitial() async {
    if (_interstitialAd.isLoaded) {
      await _interstitialAd.show();
      // Reload for next time
      _loadInterstitial();
    }
  }

  @override
  void dispose() {
    _interstitialAd.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ElevatedButton(
      onPressed: _showInterstitial,
      child: Text('Show Interstitial'),
    );
  }
}
```

### Rewarded Ads

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

class MyScreen extends StatefulWidget {
  @override
  State<MyScreen> createState() => _MyScreenState();
}

class _MyScreenState extends State<MyScreen> {
  late RewardedAd _rewardedAd;

  @override
  void initState() {
    super.initState();
    _rewardedAd = RewardedAd(adUnitId: 'YOUR_REWARDED_AD_UNIT_ID');
    
    // Listen for rewards
    _rewardedAd.onReward.listen((reward) {
      print('User earned ${reward.amount} ${reward.type}');
      _grantReward(reward);
    });
    
    _loadRewarded();
  }

  Future<void> _loadRewarded() async {
    await _rewardedAd.load();
  }

  Future<void> _showRewarded() async {
    if (_rewardedAd.isLoaded) {
      final reward = await _rewardedAd.show();
      if (reward != null) {
        print('Completed! Reward: ${reward.amount} ${reward.type}');
      }
      _loadRewarded();
    }
  }

  void _grantReward(Reward reward) {
    // Grant coins, lives, etc. to user
  }

  @override
  void dispose() {
    _rewardedAd.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ElevatedButton(
      onPressed: _showRewarded,
      child: Text('Watch Ad for Reward'),
    );
  }
}
```

## Signal Collection (S2S Bidding)

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

// Collect signals from all networks
final signals = await SignalCollector.collectSignals();

// Collect from specific networks
final specificSignals = await SignalCollector.collectSignalsFromNetworks([
  'meta',
  'applovin',
  'unity',
]);

// Get device signals
final deviceSignals = await SignalCollector.getDeviceSignals();

// Send to your server for auction
final response = await http.post(
  Uri.parse('https://your-server.com/bid'),
  body: jsonEncode({
    'signals': signals.map((s) => s.toMap()).toList(),
    'device': deviceSignals.toMap(),
  }),
);
```

## Consent Management

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

// Set GDPR consent
await ConsentManager.setHasUserConsent(true);

// Set COPPA status
await ConsentManager.setIsAgeRestrictedUser(false);

// Set CCPA opt-out
await ConsentManager.setDoNotSell(false);

// Set TCF consent string
await ConsentManager.setTCFConsentString('TCF_CONSENT_STRING');

// Or set full consent status
await ConsentManager.setConsentStatus(ConsentStatus(
  hasConsent: true,
  tcfConsentString: 'TCF_STRING',
  vendorConsents: {
    'admob': true,
    'meta': true,
    'unity': false,
  },
));
```

## Adapter Health Monitoring

```dart
import 'package:adproject_sdk/adproject_sdk.dart';

// Check specific adapter
final metaHealth = await AdapterHealthManager.getAdapterHealth('meta');
print('Meta status: ${metaHealth?.status}');
print('Meta fill rate: ${metaHealth?.metrics.fillRate}%');

// Get all adapter statuses
final allHealth = await AdapterHealthManager.getAllAdapterHealth();
allHealth.forEach((network, health) {
  print('$network: ${health.status} - Ready: ${health.isReady}');
});

// Get ready adapters
final readyAdapters = await AdapterHealthManager.getReadyAdapters();
print('Ready: $readyAdapters');

// Get failed adapters
final failedAdapters = await AdapterHealthManager.getFailedAdapters();
for (final adapter in failedAdapters) {
  print('Failed: ${adapter.networkName} - ${adapter.errorMessage}');
}
```

## API Reference

### AdProjectSdk

| Method | Description |
|--------|-------------|
| `initialize(appId, {testMode, verbose})` | Initialize the SDK |
| `isInitialized` | Check if SDK is initialized |
| `setUserId(userId)` | Set user ID for tracking |
| `setUserSegments(segments)` | Set user segments |

### Banner Ads

| Class | Description |
|-------|-------------|
| `BannerAd` | Widget for displaying banner ads |
| `BannerAdController` | Imperative banner control |

### Interstitial Ads

| Method | Description |
|--------|-------------|
| `InterstitialAd.load()` | Load an interstitial |
| `InterstitialAd.show()` | Show loaded interstitial |
| `InterstitialAd.isLoaded` | Check if ready |

### Rewarded Ads

| Method | Description |
|--------|-------------|
| `RewardedAd.load()` | Load a rewarded ad |
| `RewardedAd.show()` | Show and get reward |
| `RewardedAd.onReward` | Stream of rewards |

### Ad Sizes

| Size | Dimensions |
|------|------------|
| `AdSize.banner` | 320x50 |
| `AdSize.largeBanner` | 320x100 |
| `AdSize.mediumRectangle` | 300x250 |
| `AdSize.fullBanner` | 468x60 |
| `AdSize.leaderboard` | 728x90 |
| `AdSize.adaptive(width)` | Adaptive |

## License

MIT
