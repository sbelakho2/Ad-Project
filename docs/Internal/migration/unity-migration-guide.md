# Unity to Rival ApexMediation Migration Guide

Complete migration guide for publishers transitioning from Unity LevelPlay to Rival ApexMediation.

## 🎯 Why Migrate?

| Issue | Unity LevelPlay | Rival ApexMediation |
|-------|----------------|----------------|
| **Reliability** | OTA crashes, unpredictable updates | Signed configs, staged rollouts, auto-rollback |
| **Performance** | High ANR rates (>0.1%) | <0.02% ANR guarantee |
| **Transparency** | Opaque bidding | Per-impression bid landscapes |
| **Payments** | Monthly, single rail (Tipalti issues) | Weekly, multi-rail with failover |
| **Support** | Enterprise-only priority | White-glove for all during migration |

## 📋 Pre-Migration Checklist

- [ ] **Audit current setup**
  - Document all placements and formats
  - List active ad networks
  - Export historical revenue data
  - Note custom configurations
  
- [ ] **Prepare stakeholders**
  - Get executive buy-in
  - Notify finance team of payout changes
  - Brief engineering on SDK replacement
  - Set success metrics

- [ ] **Test environment ready**
  - Staging app build configured
  - Analytics tracking in place
  - QA team briefed
  - Rollback plan documented

## 🚀 Migration Path

### Phase 1: Setup (Week 1)

#### 1.1 Create Rival Account
```bash
# Register at console.apexmediation.ee
# Or use CLI
npx @apexmediation/cli auth:login
npx @apexmediation/cli publisher:create \
  --name "Your Company" \
  --email "ops@yourcompany.com"
```

#### 1.2 Install Migration Tool
```bash
npm install -g @apexmediation/migrate
rival-migrate init
```

#### 1.3 Import Unity Configuration
```bash
# Export Unity setup
rival-migrate unity:export \
  --app-id YOUR_UNITY_APP_ID \
  --api-key YOUR_UNITY_API_KEY \
  --output unity-config.json

# Import to Apex
rival-migrate import unity-config.json
```

This automatically creates:
- ✅ Matching placements with same IDs
- ✅ Network adapter mappings
- ✅ Waterfall priorities converted to bid floors
- ✅ Segment targeting rules

### Phase 2: SDK Integration (Week 1-2)

#### 2.1 Android Migration

**Remove Unity SDK:**
```gradle
// build.gradle - REMOVE
dependencies {
    implementation 'com.unity3d.ads:unity-ads:4.x.x'
    implementation 'com.ironsource.sdk:mediationsdk:7.x.x'
}
```

**Add Rival SDK:**
```gradle
// build.gradle - ADD
dependencies {
    implementation 'com.rivalapexmediation:sdk:0.1.0'
    
    // Optional: Keep adapters for your networks
    implementation 'com.rivalapexmediation:adapter-admob:0.1.0'
    implementation 'com.rivalapexmediation:adapter-applovin:0.1.0'
}
```

**Update Initialization:**
```kotlin
// BEFORE (Unity)
IronSource.init(this, "APP_KEY", IronSource.AD_UNIT.REWARDED_VIDEO)

// AFTER (Rival)
MediationSDK.initialize(
    context = this,
    appId = "your_rival_app_id",  // From console
    config = SDKConfig.Builder()
        .testMode(BuildConfig.DEBUG)
        .build()
)
```

**Update Ad Loading:**
```kotlin
// BEFORE (Unity/IronSource)
IronSource.loadInterstitial()
IronSource.setLevelPlayInterstitialListener(object : LevelPlayInterstitialListener {
    override fun onAdReady(adInfo: AdInfo) { /* ... */ }
    override fun onAdLoadFailed(error: IronSourceError) { /* ... */ }
    // ...
})

// AFTER (Rival)
MediationSDK.getInstance().loadAd(
    placement = "interstitial_main",
    callback = object : AdLoadCallback {
        override fun onAdLoaded(ad: Ad) {
            ad.show(this@MainActivity)
        }
        override fun onError(error: AdError, message: String) {
            Log.e("Ad", "Failed: $message")
        }
    }
)
```

#### 2.2 iOS Migration

**Remove Unity SDK:**
```ruby
# Podfile - REMOVE
pod 'IronSourceSDK', '~> 7.5.0'
pod 'UnityAds', '~> 4.9.0'
```

**Add Rival SDK:**
```ruby
# Podfile - ADD
pod 'RivalApexMediation', '~> 0.1.0'
pod 'RivalApexMediation-AdMob', '~> 0.1.0'  # Optional adapters
pod 'RivalApexMediation-AppLovin', '~> 0.1.0'
```

**Update Initialization:**
```swift
// BEFORE (Unity)
IronSource.initWithAppKey("APP_KEY")

// AFTER (Rival)
MediationSDK.initialize(
    appId: "your_rival_app_id",
    config: SDKConfig.builder()
        .testMode(true)
        .build()
)
```

**Update Ad Loading:**
```swift
// BEFORE (Unity/IronSource)
IronSource.loadInterstitial()
// Delegate methods...

// AFTER (Rival)
MediationSDK.shared.loadAd(
    placement: "interstitial_main"
) { result in
    switch result {
    case .success(let ad):
        ad.show(from: self)
    case .failure(let error):
        print("Failed to load ad: \(error)")
    }
}
```

#### 2.3 Unity Engine Migration

**Remove Unity Ads Package:**
```bash
# Remove via Package Manager or
rm -rf Packages/com.unity.ads
rm -rf Packages/com.unity.mediation
```

**Add Rival Unity Package:**
```bash
# Import via Package Manager
# Or download from https://github.com/rivalapexmediation/unity-sdk/releases

# Add to manifest.json
{
  "dependencies": {
    "com.rivalapexmediation.mediation": "0.1.0"
  }
}
```

**Update Code:**
```csharp
// BEFORE (Unity Ads)
using UnityEngine.Advertisements;

Advertisement.Initialize("GAME_ID");
Advertisement.Load("Interstitial_Android");
Advertisement.Show("Interstitial_Android");

// AFTER (Rival)
using RivalApexMediation;

RivalMediation.Initialize("your_rival_app_id");
RivalMediation.LoadAd("interstitial_main", (ad, error) => {
    if (ad != null) {
        ad.Show();
    }
});
```

### Phase 3: Testing (Week 2)

#### 3.1 Automated Tests
```bash
# Run SDK integration tests
npm run test:sdk

# Run migration validation
rival-migrate validate \
  --placement-coverage \
  --adapter-mapping \
  --revenue-parity
```

#### 3.2 Manual QA Checklist
- [ ] All placement IDs load correctly
- [ ] Ads display properly on all form factors
- [ ] No memory leaks (test 100+ ad loads)
- [ ] No ANR/crashes after 1000 impressions
- [ ] Analytics events tracking correctly
- [ ] Viewability measurements accurate

#### 3.3 A/B Testing Setup
```javascript
// Option 1: Feature flag (recommended)
if (featureFlags.rivalApexMediation) {
    // Use Rival SDK
} else {
    // Keep Unity as fallback
}

// Option 2: Traffic split
const userId = getUserId();
if (userId % 10 < 2) {  // 20% traffic
    initRivalSDK();
} else {
    initUnitySDK();
}
```

### Phase 4: Staged Rollout (Week 3-4)

#### Stage 1: 5% Traffic (Days 1-3)
```bash
# Update Firebase Remote Config or LaunchDarkly
rival_sdk_enabled: true
rival_sdk_percentage: 5

# Monitor metrics
rival-migrate monitor \
  --metric revenue_per_user \
  --metric fill_rate \
  --metric crash_rate \
  --baseline unity \
  --test rival \
  --duration 72h
```

**Success Criteria:**
- Revenue per user >= 95% of Unity baseline
- Fill rate >= 98% of Unity baseline
- Crash rate < Unity baseline
- No ANR events

#### Stage 2: 25% Traffic (Days 4-7)
```bash
rival_sdk_percentage: 25

# Continue monitoring
rival-migrate monitor --duration 96h
```

#### Stage 3: 100% Traffic (Days 8-14)
```bash
rival_sdk_percentage: 100

# Full migration monitoring
rival-migrate monitor --duration 168h --final-check
```

### Phase 5: Unity Removal (Week 4-5)

#### 5.1 Verify Full Migration
```bash
# Check that 100% traffic on Rival
rival-migrate status

# Export final metrics comparison
rival-migrate report \
  --format pdf \
  --output migration-success-report.pdf
```

#### 5.2 Remove Unity SDK
```bash
# Android
# Remove Unity dependencies from build.gradle

# iOS  
# Remove Unity pods from Podfile
pod deintegrate
pod install

# Unity
# Delete Unity packages
```

#### 5.3 Update Payment Settings
```bash
# In Rival console, configure payout preferences
# Set minimum threshold
# Choose primary rail (Adyen/PayPal/Wire)
# Add backup rails
```

## 🔧 Code Migration Patterns

### Placement Mapping
```javascript
// Unity placement IDs → Rival placement IDs
const PLACEMENT_MAP = {
  'video': 'rewarded_video_main',
  'Rewarded_Android': 'rewarded_video_main',
  'Interstitial_Android': 'interstitial_main',
  'banner': 'banner_footer'
};

function getRivalPlacement(unityPlacement) {
  return PLACEMENT_MAP[unityPlacement] || unityPlacement;
}
```

### Adapter Name Mapping
```javascript
const ADAPTER_MAP = {
  'AdMob': 'admob',
  'AppLovin': 'applovin',
  'Facebook': 'meta_audience_network',
  'IronSource': 'is_exchange',
  'UnityAds': 'unity_exchange'
};
```

### Event Tracking Migration
```javascript
// BEFORE (Unity Analytics)
Advertisement.AddListener({
  onUnityAdsReady: (placementId) => {
    analytics.track('unity_ad_ready', { placement: placementId });
  },
  onUnityAdsDidFinish: (placementId, result) => {
    analytics.track('unity_ad_finish', { 
      placement: placementId,
      result: result
    });
  }
});

// AFTER (Rival Analytics)
RivalMediation.setEventListener({
  onAdLoaded: (placement, network) => {
    analytics.track('rival_ad_loaded', { 
      placement: placement,
      network: network
    });
  },
  onAdShown: (placement, network, revenue) => {
    analytics.track('rival_ad_shown', {
      placement: placement,
      network: network,
      revenue: revenue  // Actual revenue passed!
    });
  }
});
```

## 💰 Revenue Guarantee Program

During migration, Rival guarantees revenue parity:

1. **Baseline Period**: We measure your Unity revenue (last 30 days)
2. **Migration Period**: First 30 days on Rival
3. **Comparison**: If Rival revenue < Unity baseline, we compensate the difference

**Eligibility:**
- Must follow migration guide
- Must complete A/B testing phase
- Must provide Unity analytics export

**To enroll:**
```bash
rival-migrate guarantee:enroll \
  --unity-revenue-csv unity-last-30days.csv
```

## 🆘 Troubleshooting

### Issue: Lower fill rate on Rival

**Diagnosis:**
```bash
rival-migrate diagnose fill-rate \
  --placement interstitial_main
```

**Common Causes:**
1. Network adapters not configured
2. Bid floors too high
3. Geography not covered

**Solution:**
```bash
# Check adapter status
rival-migrate adapters:list --status

# Adjust bid floors
rival-migrate floors:optimize --placement interstitial_main
```

### Issue: SDK crashes

**Diagnosis:**
```bash
# Export crash logs
rival-migrate logs:export --type crash --last 24h

# Check for known issues
rival-migrate known-issues --sdk-version 0.1.0
```

**Solution:**
- Update to latest SDK version
- Check ProGuard rules (Android)
- Verify minimum OS versions

### Issue: Revenue discrepancy

**Diagnosis:**
```bash
rival-migrate revenue:compare \
  --start 2025-01-01 \
  --end 2025-01-31 \
  --unity unity-revenue.csv
```

**Common Causes:**
1. Different attribution windows
2. Currency conversion differences
3. Fraud filtering (Rival filters more aggressively)

## 📞 Support During Migration

**White-Glove Migration Support:**
- Dedicated Slack channel
- Weekly check-in calls
- 24/7 emergency hotline
- Direct engineering access

**Contact:**
- Email: support@apexmediation.ee
- Slack: rivalapexmediation.slack.com
- Emergency: +1-XXX-XXX-XXXX

## 📊 Success Metrics

Track these throughout migration:

| Metric | Target | Measurement |
|--------|--------|-------------|
| Revenue per DAU | >= Unity baseline | Analytics |
| Fill rate | >= 98% | SDK telemetry |
| eCPM | >= Unity average | Reporting API |
| Crash rate | < 0.2% | Crash reporting |
| ANR rate | < 0.02% | Android vitals |
| Migration time | < 4 weeks | Timeline tracking |

## 🎓 Training Resources

- **Video Tutorials**: https://apexmediation.ee/docs/videos
- **Live Workshops**: Bi-weekly on Wednesdays
- **Sample Apps**: https://github.com/rivalapexmediation/samples
- **API Reference**: https://apexmediation.ee/docs/api

## Next Steps

1. ✅ Complete pre-migration checklist
2. 📝 Schedule kickoff call: https://calendly.com/apexmediation/migration
3. 🚀 Begin Phase 1 setup
4. 📊 Monitor daily metrics
5. 🎉 Celebrate successful migration!
