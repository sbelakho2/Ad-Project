# Quick Start Guide

Welcome to the ApexMediation Platform! This guide will help you get started in under 10 minutes.

## Prerequisites

- Active account (sign up at [apexmediation.bel-consulting.ee](https://apexmediation.bel-consulting.ee))
- A mobile app or website where you want to show ads
- Basic programming knowledge

## Step 1: Create Your Account

1. Go to [apexmediation.bel-consulting.ee/signup](https://apexmediation.bel-consulting.ee/signup)
2. Fill in your details:
   - Email address
   - Company name
   - Password
3. Verify your email
4. Complete your profile in the dashboard

**⏱️ Time: 3 minutes**

## Step 2: Create Your First App

1. Log in to the [dashboard](https://apexmediation.bel-consulting.ee/dashboard)
2. Click "Add New App"
3. Fill in app details:
   - App name (e.g., "My Awesome Game")
   - Platform (iOS, Android, Unity, Web)
   - Bundle ID / Package name
   - App store URL (optional)
4. Click "Create App"
5. **Save your API Key** - you'll need this for integration

**⏱️ Time: 2 minutes**

## Step 3: Integrate the SDK

Choose your platform:

### Unity (Recommended for Games)

1. In **Package Manager** select **Add package from git URL...** and paste `https://github.com/sbelakho2/Ad-Project.git?path=sdk/core/unity#main`.
2. Open **Apex Mediation > Config-as-Code**, export a signed config JSON, and save it as a `TextAsset` (e.g., `Assets/Config/apex_config.json`).
3. Drop the **Apex Mediation Entry Point** component onto a bootstrap GameObject, assign the TextAsset + signing key, and enable "Attach Debugger Overlay in Editor" for local runs.
4. Drive everything else through the static facade:

```csharp
using Apex.Mediation;

public class AdManager : MonoBehaviour
{
    void Start()
    {
        ApexMediation.LoadInterstitial("home_screen");
    }

    public void ShowInterstitial()
    {
        ApexMediation.ShowInterstitial("home_screen");
    }
}
```

**📖 Full Guide**: [Unity SDK Documentation](/docs/integration-guides/unity-sdk)

### iOS (Swift)

1. Install via Swift Package Manager:
   ```
    https://github.com/sbelakho2/Ad-Project.git
   ```

2. Initialize in `AppDelegate`:

```swift
import ApexMediationSDK

func application(_ application: UIApplication, 
                 didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
    ApexMediation.initialize(apiKey: "YOUR_API_KEY")
    return true
}
```

3. Show a banner ad:

```swift
import ApexMediationSDK

class ViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        
        let bannerAd = ApexMediationBannerView(adUnitId: "home_screen")
        bannerAd.frame = CGRect(x: 0, y: view.bounds.height - 50, 
                                width: view.bounds.width, height: 50)
        view.addSubview(bannerAd)
        bannerAd.load()
    }
}
```

**📖 Full Guide**: [iOS SDK Documentation](/docs/integration-guides/ios-sdk)

### Android (Kotlin)

1. Add to `build.gradle`:

```gradle
dependencies {
    implementation 'com.belconsulting:apexmediation-android:1.0.0'
}
```

2. Initialize in `Application` class:

```kotlin
import com.belconsulting.apexmediation.ApexMediation

class MyApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        ApexMediation.initialize(this, "YOUR_API_KEY")
    }
}
```

3. Show a banner ad:

```kotlin
import com.belconsulting.apexmediation.BannerAdView

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        
        val bannerAd = findViewById<BannerAdView>(R.id.banner_ad)
        bannerAd.adUnitId = "home_screen"
        bannerAd.load()
    }
}
```

**📖 Full Guide**: [Android SDK Documentation](/docs/integration-guides/android-sdk)

### Web (JavaScript)

1. Add the script tag to your HTML:

```html
<script src="https://cdn.apexmediation.ee/sdk/v1/apexmediation.min.js"></script>
<script>
  ApexMediation.init({ apiKey: 'YOUR_API_KEY' });
</script>
```

2. Create an ad container:

```html
<div id="banner-ad" data-apexmediation-unit="home_screen"></div>
```

3. Load the ad:

```javascript
ApexMediation.loadAd('banner-ad');
```

**📖 Full Guide**: [Web SDK Documentation](/docs/integration-guides/web-sdk)

**⏱️ Time: 5 minutes**

## Step 4: Test Your Integration

1. Run your app in development/test mode
2. Check the dashboard for test impressions
3. Verify ads are displaying correctly
4. Check the console for any errors

**Common Issues:**
- **No ads showing?** Check your API key is correct
- **Console errors?** See [Troubleshooting Guide](/docs/troubleshooting/common-issues)
- **Need help?** Contact support@apexmediation.ee

**⏱️ Time: 2 minutes**

## Step 5: Go Live

1. In the dashboard, switch your app from "Test Mode" to "Live Mode"
2. Submit your app to App Store / Google Play
3. Monitor performance in the [Analytics Dashboard](/dashboard/analytics)

**⏱️ Time: 1 minute**

---

## What's Next?

### 📊 Optimize Your Revenue
- [Set up A/B testing](/docs/features/ab-testing) to find the best ad placements
- [Enable fraud detection](/docs/features/fraud-detection) to protect your revenue
- [Configure mediation](/docs/features/mediation) to maximize eCPM

### 🎯 Advanced Features
- [Custom bidding algorithms](/docs/advanced/custom-bidding)
- [Geographic discount optimization](/docs/features/geographic-discounts)
- [Self-evolving AI system](/docs/advanced/self-evolving-ai)

### 💰 Get Paid
- [Set up your payment details](/docs/billing-compliance/payment-terms)
- [Understand Estonian tax compliance](/docs/billing-compliance/estonian-tax)
- [Download invoices](/dashboard/payments)

---

## Support

- **📧 Email**: support@apexmediation.ee
- **💬 Live Chat**: Available in the dashboard
- **📚 Documentation**: [docs.apexmediation.ee](https://docs.apexmediation.ee)
- **🐛 Report a Bug**: [GitHub Issues](https://github.com/sbelakho2/Ad-Project/issues)
- **⏱️ Response Time**: < 4 hours during business hours (Mon-Fri, 9AM-5PM EET)

---

**Estimated Total Time: 13 minutes**

**Ready to scale?** 🚀 Check out our [Scaling Guidelines](/docs/advanced/scaling) for apps with 1M+ daily active users.
