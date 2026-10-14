package com.rivalapexmediation.sdk;

import android.content.Context;
import androidx.test.core.app.ApplicationProvider;
import com.rivalapexmediation.sdk.models.Ad;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;

import static org.junit.Assert.*;

/**
 * Java interop smoke test ensuring public Kotlin APIs work seamlessly from Java code.
 * Tests SDK initialization and basic facade usage (load/show/isReady) without relying on Kotlin-specific features.
 *
 * Acceptance:
 * - SDKConfig construction and builder pattern work from Java
 * - BelAds.initialize() can be called from Java
 * - BelInterstitial load()/show()/isReady() compile and execute from Java
 * - Callbacks work with Java anonymous classes
 */
@RunWith(RobolectricTestRunner.class)
public class JavaInteropSmoke {

    private Context context;

    @Before
    public void setUp() {
        context = ApplicationProvider.getApplicationContext();
    }

    @Test
    public void sdkConfig_canBeConstructedFromJava() {
        // Default constructor with builder
        SDKConfig config = new SDKConfig.Builder()
                .appId("test-app-id")
                .testMode(true)
                .logLevel(LogLevel.DEBUG)
                .build();

        assertNotNull("SDKConfig should be instantiated", config);
        assertEquals("App ID should match", "test-app-id", config.getAppId());
        assertTrue("Test mode should be enabled", config.getTestMode());
        assertEquals("Log level should be DEBUG", LogLevel.DEBUG, config.getLogLevel());
    }

    @Test
    public void sdkConfig_defaultConstructor_worksFromJava() {
        // Test @JvmOverloads on data class constructor
        SDKConfig config = new SDKConfig(
                "app-id",
                false,
                LogLevel.INFO,
                true,
                "https://config.example.com",
                "https://auction.example.com",
                null,               // auctionApiKey (nullable String)
                false,              // strictModePenaltyDeath
                null,               // configPublicKeyBase64 (nullable String)
                5,
                60000L,
                3
        );

        assertNotNull(config);
        assertEquals("app-id", config.getAppId());
    }

    @Test
    public void belAds_initialize_worksFromJava() {
        // Test @JvmOverloads on BelAds.initialize()
        MediationSDK sdk = BelAds.initialize(context, "test-app-java");
        assertNotNull("SDK should be initialized", sdk);

        // With explicit config
        SDKConfig config = new SDKConfig.Builder()
                .appId("test-app-java-2")
                .testMode(true)
                .build();
        MediationSDK sdk2 = BelAds.initialize(context, "test-app-java-2", config);
        assertNotNull("SDK with config should be initialized", sdk2);
    }

    @Test
    public void belAds_setConsent_worksFromJava() {
        // Test @JvmOverloads on BelAds.setConsent() - all parameters optional
        BelAds.setConsent(
                "tcString123",      // tcString
                "usPrivacy456",     // usPrivacy
                null,               // gpp
                true,               // gdprApplies
                false,              // coppa
                null,               // ldu
                false               // limitAdTracking
        );

        // Call with nulls (default parameters)
        BelAds.setConsent(null, null, null, null, null, null, null);
    }

    @Test
    public void belInterstitial_load_worksFromJava() {
        BelAds.initialize(context, "test-app-java");

        final boolean[] callbackFired = {false};

        // Java anonymous class implementing AdLoadCallback
        AdLoadCallback callback = new AdLoadCallback() {
            @Override
            public void onAdLoaded(Ad ad) {
                callbackFired[0] = true;
                assertNotNull("Ad should not be null", ad);
            }

            @Override
            public void onError(AdError error, String message) {
                // Expected in this test (no real backend)
                callbackFired[0] = true;
                assertNotNull("Error should not be null", error);
            }
        };

        // This should compile without issues
        BelInterstitial.load(context, "placement-java-test", callback);

        // Note: actual callback invocation requires background thread execution
        // which is complex in Robolectric; this test verifies compilation and API shape
    }

    @Test
    public void belInterstitial_isReady_worksFromJava() {
        BelAds.initialize(context, "test-app-java");

        // Should return false before loading
        boolean ready = BelInterstitial.isReady();
        assertFalse("Interstitial should not be ready before load", ready);
    }

    @Test
    public void belRewarded_apiCallsCompileFromJava() {
        BelAds.initialize(context, "test-app-java");

        AdLoadCallback callback = new AdLoadCallback() {
            @Override
            public void onAdLoaded(Ad ad) {}

            @Override
            public void onError(AdError error, String message) {}
        };

        // Verify API compiles from Java
        BelRewarded.load(context, "placement-rewarded", callback);
        boolean ready = BelRewarded.isReady();
        assertFalse(ready);
    }

    @Test
    public void belBanner_apiCallsCompileFromJava() {
        BelAds.initialize(context, "test-app-java");

        // Verify static methods compile
        assertNotNull("BelBanner class should be accessible", BelBanner.class);
        // Note: attach/detach require ViewGroup which is complex to setup in unit tests
    }

    @Test
    public void belAppOpen_apiCallsCompileFromJava() {
        BelAds.initialize(context, "test-app-java");

        AdLoadCallback callback = new AdLoadCallback() {
            @Override
            public void onAdLoaded(Ad ad) {}

            @Override
            public void onError(AdError error, String message) {}
        };

        BelAppOpen.load(context, "placement-app-open", callback);
        boolean ready = BelAppOpen.isReady();
        assertFalse(ready);
    }

    @Test
    public void belRewardedInterstitial_apiCallsCompileFromJava() {
        BelAds.initialize(context, "test-app-java");

        AdLoadCallback callback = new AdLoadCallback() {
            @Override
            public void onAdLoaded(Ad ad) {}

            @Override
            public void onError(AdError error, String message) {}
        };

        BelRewardedInterstitial.load(context, "placement-rewarded-int", callback);
        boolean ready = BelRewardedInterstitial.isReady();
        assertFalse(ready);
    }

    @Test
    public void belAds_utilityMethods_workFromJava() {
        BelAds.initialize(context, "test-app-java");

        // Test runtime methods
        BelAds.setTestMode(true);
        BelAds.setLogLevel(LogLevel.VERBOSE);
        BelAds.registerTestDevice("test-device-java");

        // These should not throw
        assertTrue("Java interop test completed", true);
    }

    @Test
    public void adError_enum_accessibleFromJava() {
        // Verify enums are accessible
        AdError noFill = AdError.NO_FILL;
        AdError timeout = AdError.TIMEOUT;
        AdError networkError = AdError.NETWORK_ERROR;
        AdError internalError = AdError.INTERNAL_ERROR;
        AdError invalidPlacement = AdError.INVALID_PLACEMENT;

        assertNotNull(noFill);
        assertNotNull(timeout);
        assertNotNull(networkError);
        assertNotNull(internalError);
        assertNotNull(invalidPlacement);
    }

    @Test
    public void logLevel_enum_accessibleFromJava() {
        // Verify all log levels accessible
        LogLevel verbose = LogLevel.VERBOSE;
        LogLevel debug = LogLevel.DEBUG;
        LogLevel info = LogLevel.INFO;
        LogLevel warn = LogLevel.WARN;
        LogLevel error = LogLevel.ERROR;

        assertNotNull(verbose);
        assertNotNull(debug);
        assertNotNull(info);
        assertNotNull(warn);
        assertNotNull(error);
    }
}
