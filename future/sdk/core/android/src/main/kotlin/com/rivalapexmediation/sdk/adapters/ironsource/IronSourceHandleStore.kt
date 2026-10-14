package com.rivalapexmediation.sdk.adapters.ironsource

import com.rivalapexmediation.sdk.contract.AdHandle
import com.rivalapexmediation.sdk.contract.AdapterError
import com.rivalapexmediation.sdk.contract.ErrorCode
import com.rivalapexmediation.sdk.contract.LoadResult
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.util.ClockProvider
import java.util.concurrent.ConcurrentHashMap

internal data class IronSourceAd(
    val handle: AdHandle,
    val adType: AdType,
    val partnerPlacementId: String,
    val markup: String,
    val ttlMs: Long,
    val priceMicros: Long?,
    val currency: String?,
    val partnerMeta: Map<String, Any?>,
    val createdAtMs: Long = ClockProvider.clock.monotonicNow()
) {
    fun isFresh(nowMs: Long): Boolean = nowMs < createdAtMs + ttlMs
}

internal class IronSourceHandleStore {
    private val entries = ConcurrentHashMap<String, IronSourceAd>()

    fun put(ad: IronSourceAd) {
        entries[ad.handle.id] = ad
    }

    fun get(handle: AdHandle): IronSourceAd? = entries[handle.id]

    fun remove(handle: AdHandle) {
        entries.remove(handle.id)
    }

    fun buildLoadResult(ad: IronSourceAd): LoadResult = LoadResult(
        handle = ad.handle,
        ttlMs = ad.ttlMs.toInt(),
        priceMicros = ad.priceMicros,
        currency = ad.currency,
        partnerMeta = ad.partnerMeta
    )

    fun validateReadiness(handle: AdHandle): Boolean {
        val entry = entries[handle.id] ?: return false
        return entry.isFresh(ClockProvider.clock.monotonicNow())
    }

    fun ensureReady(handle: AdHandle): IronSourceAd {
        val ad = entries[handle.id]
            ?: throw AdapterError.Recoverable(ErrorCode.NO_AD_READY, "Handle ${handle.id} missing")
        if (!ad.isFresh(ClockProvider.clock.monotonicNow())) {
            remove(handle)
            throw AdapterError.Recoverable(ErrorCode.NO_AD_READY, "Handle ${handle.id} expired")
        }
        return ad
    }
}
