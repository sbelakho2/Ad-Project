package com.rivalapexmediation.sdk.measurement

import android.app.Activity
import android.content.Context
import android.view.View
import com.rivalapexmediation.sdk.BuildConfig
import com.rivalapexmediation.sdk.models.FeatureFlags
import com.rivalapexmediation.sdk.telemetry.TelemetryCollector
import java.util.concurrent.ConcurrentHashMap

/**
 * Reflection-only OM SDK bridge that is safe to ship when the host app does not bundle OMSDK.
 * - Never hard-links vendor classes; all lookups are best-effort.
 * - No-ops on failures; avoids ClassNotFound / linkage errors in BYO setups.
 */
object OmSdkHelper {
    private const val LIB_PREFIX = "com.iab.omid.library.apex"

    private fun enumValue(enumClass: Class<*>, name: String): Any? {
        return try {
            @Suppress("UNCHECKED_CAST")
            java.lang.Enum.valueOf(enumClass as Class<out Enum<*>>, name)
        } catch (_: Throwable) {
            null
        }
    }

    @Volatile private var initialized = false
    @Volatile private var sdkAvailable = false
    @Volatile private var partner: Any? = null

    @JvmStatic
    fun initIfAvailable(
        context: Context,
        partnerName: String = "ApexMediation",
        partnerVersion: String = BuildConfig.SDK_VERSION,
    ): Boolean {
        if (initialized) return sdkAvailable

        sdkAvailable = try {
            Class.forName("$LIB_PREFIX.Omid")
            true
        } catch (_: Throwable) {
            false
        }

        if (!sdkAvailable) {
            initialized = true
            return false
        }

        try {
            val omidClass = Class.forName("$LIB_PREFIX.Omid")
            val isActive = (omidClass.getMethod("isActive").invoke(null) as? Boolean) ?: false
            if (!isActive) {
                omidClass.getMethod("activate", Context::class.java)
                    .invoke(null, context.applicationContext)
            }

            val partnerClass = Class.forName("$LIB_PREFIX.adsession.Partner")
            partner = partnerClass
                .getMethod("createPartner", String::class.java, String::class.java)
                .invoke(null, partnerName, partnerVersion)
            sdkAvailable = partner != null
        } catch (_: Throwable) {
            sdkAvailable = false
        } finally {
            initialized = true
        }
        return sdkAvailable
    }

    @JvmStatic
    fun isAvailable(): Boolean = sdkAvailable && partner != null

    data class SessionHandle(
        val adSession: Any,
        val adSessionClass: Class<*>,
        val adEvents: Any?
    )

    @JvmStatic
    fun startSession(
        adView: View,
        isVideo: Boolean,
        friendlyObstructions: List<View> = emptyList(),
    ): SessionHandle? {
        if (!isAvailable()) return null
        return try {
            val adSessionConfigClass = Class.forName("$LIB_PREFIX.adsession.AdSessionConfiguration")
            val adSessionContextClass = Class.forName("$LIB_PREFIX.adsession.AdSessionContext")
            val adSessionClass = Class.forName("$LIB_PREFIX.adsession.AdSession")
            val creativeTypeClass = Class.forName("$LIB_PREFIX.adsession.CreativeType")
            val impressionTypeClass = Class.forName("$LIB_PREFIX.adsession.ImpressionType")
            val ownerClass = Class.forName("$LIB_PREFIX.adsession.Owner")
            val adEventsClass = Class.forName("$LIB_PREFIX.adsession.AdEvents")
            val friendlyPurposeClass = Class.forName("$LIB_PREFIX.adsession.FriendlyObstructionPurpose")
            val partnerClass = Class.forName("$LIB_PREFIX.adsession.Partner")

            val creativeType: Any = enumValue(creativeTypeClass, if (isVideo) "VIDEO" else "HTML_DISPLAY") ?: return null
            val impressionType: Any = enumValue(impressionTypeClass, "VIEWABLE") ?: return null
            val owner: Any = enumValue(ownerClass, "NATIVE") ?: return null
            val obstructionPurpose: Any = enumValue(friendlyPurposeClass, "OTHER") ?: return null

            val config = adSessionConfigClass.getMethod(
                "createAdSessionConfiguration",
                creativeTypeClass,
                impressionTypeClass,
                ownerClass,
                ownerClass,
                java.lang.Boolean.TYPE
            ).invoke(null, creativeType, impressionType, owner, owner, false)

            val adSessionContext = adSessionContextClass.getMethod(
                "createNativeAdSessionContext",
                partnerClass,
                View::class.java,
                String::class.java,
                String::class.java,
                String::class.java,
                String::class.java,
            ).invoke(null, partner, adView, null, null, null, null)

            val adSession = adSessionClass.getMethod(
                "createAdSession",
                adSessionConfigClass,
                adSessionContextClass
            ).invoke(null, config, adSessionContext)

            adSessionClass.getMethod("registerAdView", View::class.java).invoke(adSession, adView)

            if (friendlyObstructions.isNotEmpty()) {
                val addFo = adSessionClass.getMethod(
                    "addFriendlyObstruction",
                    View::class.java,
                    friendlyPurposeClass,
                    String::class.java,
                )
                friendlyObstructions.forEach { view ->
                    try { addFo.invoke(adSession, view, obstructionPurpose, "Overlay UI") } catch (_: Throwable) {}
                }
            }

            adSessionClass.getMethod("start").invoke(adSession)
            val adEvents = adEventsClass.getMethod("createAdEvents", adSessionClass)
                .invoke(null, adSession)
            try { adEvents::class.java.methods.firstOrNull { it.name == "loaded" }?.invoke(adEvents) } catch (_: Throwable) {}
            try { adEvents::class.java.methods.firstOrNull { it.name == "impressionOccurred" }?.invoke(adEvents) } catch (_: Throwable) {}
            SessionHandle(adSession, adSessionClass, adEvents)
        } catch (_: Throwable) {
            null
        }
    }

    @JvmStatic
    fun finishSession(handle: SessionHandle?) {
        if (handle == null) return
        try {
            handle.adSessionClass.getMethod("finish").invoke(handle.adSession)
        } catch (_: Throwable) {
        }
    }
}

/**
 * OM SDK controller implementation guarded by feature flag + runtime availability.
 */
class OmSdkSessionController(
    private val telemetry: TelemetryCollector,
    private val features: FeatureFlags,
) : OmSdkController {
    private val sessions = ConcurrentHashMap<String, OmSdkHelper.SessionHandle>()

    private fun enabled(): Boolean {
        if (!features.enableOmSdk) return false
        if (!OmSdkHelper.isAvailable()) return false
        return true
    }

    override fun startDisplaySession(
        activity: Activity,
        placementId: String,
        networkName: String,
        creativeType: String?
    ) {
        startSessionInternal(activity, placementId, isVideo = false)
    }

    override fun startVideoSession(
        activity: Activity,
        placementId: String,
        networkName: String,
        durationSec: Int?
    ) {
        startSessionInternal(activity, placementId, isVideo = true)
    }

    override fun endSession(placementId: String) {
        sessions.remove(placementId)?.let { OmSdkHelper.finishSession(it) }
    }

    private fun startSessionInternal(activity: Activity, placementId: String, isVideo: Boolean) {
        if (!enabled()) return
        val rootView = activity.window?.decorView ?: return
        val handle = OmSdkHelper.startSession(rootView, isVideo)
        if (handle != null) {
            sessions[placementId] = handle
        } else {
            telemetry.recordOmSdkStatus("start_failed")
        }
    }
}
