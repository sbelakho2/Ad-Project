# AdProject React Native SDK

Official React Native wrapper for the AdProject Mediation SDK.

## Installation

```bash
npm install @adproject/react-native-sdk
# or
yarn add @adproject/react-native-sdk
```

### iOS Setup

```bash
cd ios && pod install
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

```tsx
import { AdProject } from '@adproject/react-native-sdk';

// In your App.tsx or main component
useEffect(() => {
  async function initAds() {
    const status = await AdProject.initialize('YOUR_APP_ID', {
      testMode: __DEV__,
      verbose: __DEV__,
    });
    
    console.log('SDK initialized:', status.isInitialized);
    console.log('Adapters:', status.adapters);
  }
  
  initAds();
}, []);
```

### Banner Ads

```tsx
import { BannerAd, AdSize } from '@adproject/react-native-sdk';

function MyScreen() {
  return (
    <BannerAd
      adUnitId="YOUR_BANNER_AD_UNIT_ID"
      size={AdSize.BANNER}
      onAdLoaded={(result) => console.log('Banner loaded:', result)}
      onAdFailedToLoad={(error) => console.error('Banner failed:', error)}
      onAdImpression={(data) => console.log('Revenue:', data.revenue)}
    />
  );
}
```

### Adaptive Banner

```tsx
import { BannerAd, AdSize } from '@adproject/react-native-sdk';
import { Dimensions } from 'react-native';

const { width } = Dimensions.get('window');

function MyScreen() {
  return (
    <BannerAd
      adUnitId="YOUR_BANNER_AD_UNIT_ID"
      size={AdSize.adaptive(width)}
      autoRefresh={true}
      refreshInterval={60}
    />
  );
}
```

### Interstitial Ads

```tsx
import { useInterstitialAd } from '@adproject/react-native-sdk';

function MyScreen() {
  const { isLoaded, isLoading, error, load, show } = useInterstitialAd('YOUR_INTERSTITIAL_AD_UNIT_ID');

  useEffect(() => {
    load();
  }, []);

  const handleShowAd = async () => {
    if (isLoaded) {
      await show();
      // Reload for next time
      load();
    }
  };

  return (
    <Button 
      title="Show Interstitial" 
      onPress={handleShowAd}
      disabled={!isLoaded || isLoading}
    />
  );
}
```

### Rewarded Ads

```tsx
import { useRewardedAd } from '@adproject/react-native-sdk';

function MyScreen() {
  const { isLoaded, isLoading, reward, load, show } = useRewardedAd('YOUR_REWARDED_AD_UNIT_ID');

  useEffect(() => {
    load();
  }, []);

  useEffect(() => {
    if (reward) {
      console.log(`User earned ${reward.amount} ${reward.type}`);
      // Grant the reward to the user
    }
  }, [reward]);

  return (
    <Button
      title="Watch Ad for Reward"
      onPress={show}
      disabled={!isLoaded || isLoading}
    />
  );
}
```

## Signal Collection (S2S Bidding)

For server-to-server header bidding integration:

```tsx
import { AdProject } from '@adproject/react-native-sdk';

async function collectBidSignals() {
  // Collect signals from all networks
  const signals = await AdProject.collectSignals();
  
  // Or collect from specific networks
  const specificSignals = await AdProject.collectSignals(['meta', 'applovin', 'unity']);
  
  // Send to your server for auction
  await fetch('https://your-server.com/bid', {
    method: 'POST',
    body: JSON.stringify({ signals }),
  });
}
```

## Consent Management

```tsx
import { AdProject } from '@adproject/react-native-sdk';

// Set GDPR consent
await AdProject.setHasUserConsent(true);

// Set COPPA status
await AdProject.setIsAgeRestrictedUser(false);

// Set CCPA opt-out
await AdProject.setDoNotSell(false);

// Or set full consent status
await AdProject.setConsentStatus({
  hasConsent: true,
  consentString: 'TCF_CONSENT_STRING',
  vendorConsents: {
    'admob': true,
    'meta': true,
    'unity': false,
  },
});
```

## Adapter Health Monitoring

```tsx
import { AdProject } from '@adproject/react-native-sdk';

// Check specific adapter status
const metaHealth = await AdProject.getAdapterHealth('meta');
console.log('Meta status:', metaHealth?.status);

// Get all adapter statuses
const allHealth = await AdProject.getAllAdapterHealth();
Object.entries(allHealth).forEach(([network, health]) => {
  console.log(`${network}: ${health.status}`);
});

// Get list of ready adapters
const readyAdapters = await AdProject.getReadyAdapters();
console.log('Ready adapters:', readyAdapters);
```

## API Reference

### AdProject Methods

| Method | Description |
|--------|-------------|
| `initialize(appId, options?)` | Initialize the SDK |
| `loadBannerAd(adUnitId, size?)` | Load a banner ad |
| `showBannerAd(adUnitId)` | Show a loaded banner |
| `loadInterstitialAd(adUnitId)` | Load an interstitial |
| `showInterstitialAd(adUnitId)` | Show a loaded interstitial |
| `loadRewardedAd(adUnitId)` | Load a rewarded ad |
| `showRewardedAd(adUnitId)` | Show a loaded rewarded ad |
| `collectSignals(networks?)` | Collect bid signals |
| `setConsentStatus(status)` | Set consent status |
| `getAdapterHealth(network)` | Get adapter health |

### Hooks

| Hook | Description |
|------|-------------|
| `useBannerAd(adUnitId, size?)` | Manage banner ad lifecycle |
| `useInterstitialAd(adUnitId)` | Manage interstitial ad lifecycle |
| `useRewardedAd(adUnitId)` | Manage rewarded ad lifecycle |

### Ad Sizes

| Size | Dimensions |
|------|------------|
| `AdSize.BANNER` | 320x50 |
| `AdSize.LARGE_BANNER` | 320x100 |
| `AdSize.MEDIUM_RECTANGLE` | 300x250 |
| `AdSize.FULL_BANNER` | 468x60 |
| `AdSize.LEADERBOARD` | 728x90 |
| `AdSize.adaptive(width)` | Adaptive |
| `AdSize.custom(width, height)` | Custom |

## License

MIT
