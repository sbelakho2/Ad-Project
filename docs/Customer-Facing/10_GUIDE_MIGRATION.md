# Migration Guide

## Overview

Migrating to Apex Mediation is designed to be as smooth as possible. Whether you are coming from IronSource, AppLovin MAX, or AdMob, our **Migration Studio** tool can automate much of the configuration import.

## Using Migration Studio

1.  **Export Configuration**: Export your placement and waterfall configuration from your current mediation provider (usually as a CSV or JSON file).
2.  **Upload to Apex**: Go to **Console > Migration Studio**.
3.  **Map IDs**: The tool will attempt to map your existing Ad Unit IDs to new Apex Placement IDs. Review and confirm the mappings.
4.  **Generate Config**: The tool will generate a configuration file or script to update your app.

## Manual Migration Steps

If you prefer to migrate manually or have a custom setup:

### 1. Create Apps & Placements

Recreate your app structure in the Apex Console.
*   Create an App for each of your titles.
*   Create Placements (Ad Units) for each ad spot (e.g., "Home Screen Banner", "Level End Interstitial").

### 2. Update SDK Integration

Replace your existing mediation SDK code with Apex SDK code.

**Before (IronSource Example):**
```java
IronSource.init(this, "APP_KEY");
IronSource.loadInterstitial();
```

**After (Apex):**
```kotlin
BelAds.initialize(context, "APP_ID")
// Load calls are handled via the specific ad format controllers
// e.g., BelInterstitial.load("PLACEMENT_ID")
```

### 3. Configure Networks (BYO)

Since Apex is a BYO (Bring Your Own) platform, you don't need to change your contracts with ad networks.
1.  Go to **Console > Networks**.
2.  Enable the networks you are already using (e.g., AdMob, Meta, Unity Ads).
3.  Enter your existing credentials/keys for those networks.

### 4. Test & Launch

1.  Use the **Mediation Debugger** to verify all adapters are loading.
2.  Roll out the update to a small percentage of users (e.g., 10%) to monitor stability and revenue.

## Common Pitfalls

*   **Missing Adapters**: Ensure you have added the Apex adapter for every network you want to use.
*   **ID Mismatch**: Double-check that you are using the correct Apex Placement ID, not the old provider's ID.
*   **Manifest Conflicts**: If you are removing an old SDK, ensure you also remove its entries from `AndroidManifest.xml` or `Info.plist` to avoid conflicts.
