package com.rivalapexmediation.sdk.adapters.ironsource

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import com.rivalapexmediation.sdk.contract.AdHandle
import com.rivalapexmediation.sdk.contract.AdNetworkAdapterV2
import com.rivalapexmediation.sdk.contract.AdSize
import com.rivalapexmediation.sdk.contract.AdapterConfig
import com.rivalapexmediation.sdk.contract.AdapterCredentials
import com.rivalapexmediation.sdk.contract.AdapterError
import com.rivalapexmediation.sdk.contract.AdapterOptions
import com.rivalapexmediation.sdk.contract.BannerCallbacks
import com.rivalapexmediation.sdk.contract.ErrorCode
import com.rivalapexmediation.sdk.contract.InitResult
import com.rivalapexmediation.sdk.contract.LoadResult
import com.rivalapexmediation.sdk.contract.PaidEvent
import com.rivalapexmediation.sdk.contract.PaidEventPrecision
import com.rivalapexmediation.sdk.contract.RequestMeta
import com.rivalapexmediation.sdk.contract.RewardedCallbacks
import com.rivalapexmediation.sdk.contract.ShowCallbacks
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class IronSourceAdapter(
    private val context: Context,
    private val renderer: IronSourceCreativeRenderer = IronSourceCreativeRenderer(),
    private val clock: Clock = ClockProvider.clock
) : AdNetworkAdapterV2 {

    private val initialized = AtomicBoolean(false)
    private val store = IronSourceHandleStore()
    private val apiClientRef = AtomicReference<IronSourceApiClient?>()
    private val configRef = AtomicReference<AdapterConfig?>()

    override fun init(config: AdapterConfig, timeoutMs: Int): InitResult {
        if (!config.partner.equals(PARTNER_NAME, ignoreCase = true)) {
            return InitResult(
                success = false,
                error = AdapterError.Fatal(ErrorCode.CONFIG, "Adapter misconfigured for ${config.partner}")
            )
        }

        if (config.credentials.key.isBlank()) {
            return InitResult(false, AdapterError.Fatal(ErrorCode.CONFIG, "ironSource appKey missing"))
        }

        val secret = config.credentials.secret?.takeIf { it.isNotBlank() }
            ?: return InitResult(false, AdapterError.Fatal(ErrorCode.CONFIG, "ironSource secret missing"))

        val endpoint = resolveEndpoint(config.credentials)
            ?: return InitResult(false, AdapterError.Fatal(ErrorCode.CONFIG, "ironSource endpoint missing"))

        val httpUrl = endpoint.toHttpUrlOrNull()
            ?: return InitResult(false, AdapterError.Fatal(ErrorCode.CONFIG, "Invalid endpoint: $endpoint"))

        apiClientRef.set(IronSourceApiClient(httpUrl, secret))
        configRef.set(config)
        initialized.set(true)
        return InitResult(true, partnerMeta = mapOf("endpoint" to httpUrl.toString()))
    }

    override fun loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
        return loadAd(placementId, meta, timeoutMs, AdType.INTERSTITIAL)
    }

    override fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
        val ad = store.ensureReady(handle)
        val activity = requireActivity(viewContext)
        renderer.showInterstitial(activity, ad.markup, buildPaidEvent(ad), callbacks)
        store.remove(handle)
    }

    override fun loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
        return loadAd(placementId, meta, timeoutMs, AdType.REWARDED)
    }

    override fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
        val ad = store.ensureReady(handle)
        val activity = requireActivity(viewContext)
        renderer.showRewarded(activity, ad.markup, buildPaidEvent(ad), callbacks, rewardType = "reward", rewardAmount = 1.0)
        store.remove(handle)
    }

    override fun loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int): LoadResult {
        throw AdapterError.Fatal(ErrorCode.ERROR, "ironSource banner integration pending")
    }

    override fun attachBanner(handle: AdHandle, bannerHost: Any, callbacks: BannerCallbacks) {
        throw AdapterError.Fatal(ErrorCode.ERROR, "Banner not supported yet")
    }

    override fun isAdReady(handle: AdHandle): Boolean = store.validateReadiness(handle)

    override fun expiresAt(handle: AdHandle): Long {
        return store.get(handle)?.let { it.createdAtMs + it.ttlMs } ?: 0L
    }

    override fun invalidate(handle: AdHandle) {
        store.remove(handle)
    }

    private fun loadAd(
        placementId: String,
        meta: RequestMeta,
        timeoutMs: Int,
        adType: AdType
    ): LoadResult {
        ensureInitialized()
        val cfg = configRef.get() ?: throw AdapterError.Fatal(ErrorCode.CONFIG, "Adapter not initialized")
        val partnerPlacementId = cfg.placements[placementId]
            ?: throw AdapterError.Fatal(ErrorCode.CONFIG, "Placement $placementId missing mapping")
        val appKey = cfg.credentials.key.takeIf { it.isNotBlank() }
            ?: throw AdapterError.Fatal(ErrorCode.CONFIG, "ironSource appKey missing")

        val request = buildBidRequest(partnerPlacementId, appKey, meta, cfg, adType)
        val client = apiClientRef.get() ?: throw AdapterError.Fatal(ErrorCode.CONFIG, "Missing client")
        val payload = client.loadBid(request, timeoutMs)
        val markup = payload.adMarkup?.takeIf { it.isNotBlank() }
            ?: throw AdapterError.Recoverable(ErrorCode.NO_FILL, "ironSource returned empty creative")

        val handle = AdHandle(
            id = UUID.randomUUID().toString(),
            adType = adType,
            partnerPlacementId = partnerPlacementId,
            createdAtMs = clock.monotonicNow()
        )
        val ttl = payload.ttl?.takeIf { it > 0 } ?: DEFAULT_TTL_MS
        val priceMicros = payload.revenue?.let { (it * 1_000_000L).toLong() }
        val ad = IronSourceAd(
            handle = handle,
            adType = adType,
            partnerPlacementId = partnerPlacementId,
            markup = markup,
            ttlMs = ttl,
            priceMicros = priceMicros,
            currency = DEFAULT_CURRENCY,
            partnerMeta = buildPartnerMeta(payload)
        )
        store.put(ad)
        return store.buildLoadResult(ad)
    }

    private fun buildBidRequest(
        partnerPlacementId: String,
        appKey: String,
        meta: RequestMeta,
        cfg: AdapterConfig,
        adType: AdType
    ): IronSourceBidRequest {
        val consent = cfg.privacy
        val userConsent = meta.user.consent
        val mergedConsent = IronSourceConsent(
            gdpr = userConsent.iabTcfV2 ?: consent.iabTcfV2,
            usPrivacy = userConsent.iabUsPrivacy ?: consent.iabUsPrivacy,
            coppa = userConsent.coppa || consent.coppa
        )
        val advertisingId = if (userConsent.limitAdTracking) {
            null
        } else {
            meta.user.advertisingId ?: userConsent.advertisingId
        }
        val floor = chooseFloor(meta, cfg.options)
        val adUnit = when (adType) {
            AdType.INTERSTITIAL -> "interstitial"
            AdType.REWARDED -> "rewarded"
            else -> "interstitial"
        }
        return IronSourceBidRequest(
            appKey = appKey,
            instanceId = partnerPlacementId,
            adUnit = adUnit,
            bundleId = cfg.credentials.appId?.takeIf { it.isNotBlank() } ?: context.packageName,
            platform = meta.device.os,
            osVersion = meta.device.osVersion,
            deviceModel = meta.device.model,
            ip = meta.net.ipPrefixed,
            lmt = userConsent.limitAdTracking,
            userAgent = meta.net.uaNormalized,
            sessionDepth = meta.context.sessionDepth,
            requestId = meta.requestId,
            floorMicros = floor,
            test = cfg.options?.testMode == true,
            consent = mergedConsent,
            country = cfg.region?.name,
            advertisingId = advertisingId
        )
    }

    private fun chooseFloor(meta: RequestMeta, options: AdapterOptions?): Long? {
        val requestFloor = meta.auction.floorsMicros
        val adapterFloor = options?.bidFloorMicros
        return when {
            requestFloor == null && adapterFloor == null -> null
            requestFloor == null -> adapterFloor
            adapterFloor == null -> requestFloor
            else -> maxOf(requestFloor, adapterFloor)
        }
    }

    private fun ensureInitialized() {
        if (!initialized.get()) {
            throw AdapterError.Fatal(ErrorCode.CONFIG, "ironSource adapter not initialized")
        }
    }

    private fun resolveEndpoint(credentials: AdapterCredentials): String? {
        return credentials.accountIds?.get("endpoint")?.takeIf { it.isNotBlank() } ?: DEFAULT_ENDPOINT
    }

    private fun requireActivity(viewContext: Any): Activity {
        return when (viewContext) {
            is Activity -> viewContext
            is ContextWrapper -> viewContext.baseContext as? Activity
            is Context -> viewContext as? Activity
            else -> null
        } ?: throw AdapterError.Fatal(ErrorCode.ERROR, "ironSource requires Activity context for rendering")
    }

    private fun buildPartnerMeta(payload: IronSourceBidResponse): Map<String, Any?> {
        return mapOf(
            "provider" to payload.providerName,
            "auction_id" to payload.auctionId,
            "instance_id" to payload.instanceId,
            "creative_id" to payload.creativeId
        )
    }

    private fun buildPaidEvent(ad: IronSourceAd): PaidEvent? {
        val price = ad.priceMicros ?: return null
        return PaidEvent(
            valueMicros = price,
            currency = ad.currency ?: DEFAULT_CURRENCY,
            precision = PaidEventPrecision.PUBLISHER,
            partner = PARTNER_NAME,
            partnerUnitId = ad.partnerPlacementId,
            creativeId = ad.partnerMeta["creative_id"] as? String
        )
    }

    companion object {
        internal const val DEFAULT_ENDPOINT = "https://outcome-ssp.supersonicads.com/mediation"
        private const val DEFAULT_TTL_MS = 60_000L
        private const val DEFAULT_CURRENCY = "USD"
        private const val PARTNER_NAME = "ironsource"
    }
}
