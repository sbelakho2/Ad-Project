package com.rivalapexmediation.sdk

import android.app.Activity
import android.content.Context
import com.rivalapexmediation.sdk.consent.UmpConsentClient
import com.rivalapexmediation.sdk.logging.Logger
import com.rivalapexmediation.sdk.debug.DebugPanel

/**
 * Tiny, developer-friendly public facade for the SDK.
 * Maps to the internal MediationSDK and keeps API surface minimal and boring.
 */
object BelAds {
    /** Initialize the SDK. Safe to call multiple times (idempotent). */
    @JvmStatic
    @JvmOverloads
    fun initialize(context: Context, appId: String, config: SDKConfig = SDKConfig.Builder().appId(appId).build()): MediationSDK {
        val sdk = MediationSDK.initialize(context, appId, config)
        Logger.setLevel(config.logLevel)
        return sdk
    }

    /** Optional: inject an OM SDK controller implementation for measurement (no-op by default). */
    @JvmStatic
    fun setOmSdkController(controller: com.rivalapexmediation.sdk.measurement.OmSdkController) {
        com.rivalapexmediation.sdk.measurement.OmSdkRegistry.controller = controller
    }

    /** Runtime toggle for test mode; affects S2S auction metadata and logging hints. */
    @JvmStatic
    fun setTestMode(enabled: Boolean) {
        try {
            MediationSDK.getInstance().setTestModeOverride(enabled)
            if (!enabled && BuildConfig.DEBUG) {
                Logger.w("BelAds", "Test mode disabled in DEBUG build. Ensure you use official test placements/devices.")
            }
            if (enabled && !BuildConfig.DEBUG) {
                Logger.w("BelAds", "Test mode enabled in non-debug build. Do not ship test mode to production.")
            }
        } catch (_: Throwable) {}
    }

    /** Set log verbosity at runtime; sensitive fields are always redacted. */
    @JvmStatic
    fun setLogLevel(level: LogLevel) { Logger.setLevel(level) }

    /** Provide or rotate the auction API key for S2S auctions (HYBRID/MANAGED modes only). */
    @JvmStatic
    fun setAuctionApiKey(apiKey: String) {
        try { MediationSDK.getInstance().setAuctionApiKey(apiKey) } catch (_: Throwable) {}
    }

    /** Explicit consent source of truth. */
    @JvmStatic
    @JvmOverloads
    fun setConsent(
        tcString: String? = null,
        usPrivacy: String? = null,
        @Suppress("UNUSED_PARAMETER")
        gpp: String? = null, // reserved for future use
        gdprApplies: Boolean? = null,
        coppa: Boolean? = null,
        @Suppress("UNUSED_PARAMETER")
        ldu: Boolean? = null,
        limitAdTracking: Boolean? = null,
        privacySandboxOptIn: Boolean? = null,
    ) {
        try {
            MediationSDK.getInstance().setConsent(
                gdprApplies = gdprApplies,
                consentString = tcString,
                usPrivacy = usPrivacy,
                coppa = coppa,
                limitAdTracking = limitAdTracking,
                privacySandboxOptIn = privacySandboxOptIn,
            )
        } catch (_: Throwable) {}
    }

    /** Kick off the Google UMP consent flow via the SDK. */
    @JvmStatic
    fun requestGoogleConsentForm(activity: Activity, tagForUnderAge: Boolean = false, callback: UmpConsentClient.Callback) {
        try {
            MediationSDK.getInstance().requestGoogleConsent(
                activity,
                UmpConsentClient.Params(tagForUnderAgeOfConsent = tagForUnderAge),
                callback
            )
        } catch (_: Throwable) {}
    }

    /** Refresh GAID/AppSet identifiers respecting the latest privacy settings. */
    @JvmStatic
    fun refreshPrivacyIdentifiers() {
        try { MediationSDK.getInstance().refreshPrivacyIdentifiers() } catch (_: Throwable) {}
    }

    /** Register a test device ID (e.g., hashed Android ID) for safer test ads. */
    @JvmStatic
    fun registerTestDevice(deviceId: String) {
        try { MediationSDK.getInstance().setTestDeviceId(deviceId) } catch (_: Throwable) {}
    }

    /** Show the in-app debug panel (dialog). */
    @JvmStatic
    fun showDebugPanel(activity: Activity) {
        DebugPanel.show(activity)
    }
}
