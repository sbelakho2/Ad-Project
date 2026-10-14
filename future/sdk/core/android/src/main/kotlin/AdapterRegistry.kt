package com.rivalapexmediation.sdk

import android.content.Context
import com.rivalapexmediation.sdk.contract.AdHandle
import com.rivalapexmediation.sdk.contract.AdNetworkAdapterV2
import com.rivalapexmediation.sdk.contract.InitResult
import com.rivalapexmediation.sdk.contract.LoadResult
import com.rivalapexmediation.sdk.contract.RequestMeta
import com.rivalapexmediation.sdk.contract.ShowCallbacks
import com.rivalapexmediation.sdk.contract.RewardedCallbacks
import com.rivalapexmediation.sdk.models.AdAdapter
import com.rivalapexmediation.sdk.runtime.AdapterRuntimeWrapper
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import java.util.concurrent.ConcurrentHashMap

/**
 * Minimal AdapterRegistry for the Android SDK (AdAdapter-based)
 * - Separate from sdk.core.android.src.adapter.AdapterRegistry which targets a different interface
 * - This registry serves MediationSDK by holding AdAdapter instances and lifecycle management
 */
class AdapterRegistry {
    private val adapters = ConcurrentHashMap<String, AdAdapter>()
    private val runtimeFactories = ConcurrentHashMap<String, (Context) -> AdNetworkAdapterV2>()
    private val runtimeAdapters = ConcurrentHashMap<String, RuntimeAdapterEntry>()

    init {
        // No default vendor adapters in core. Host apps (BYO) must register adapters explicitly
        // via registerRuntimeAdapterFactory() before initialize().
    }

    /**
     * Optional initialization hook; kept for API compatibility with MediationSDK.initializeInternal()
     */
    fun initialize(context: Context) {
        // Instantiate runtime adapters lazily using registered factories
        runtimeFactories.forEach { (network, factory) ->
            runtimeAdapters.computeIfAbsent(network) {
                RuntimeAdapterEntry(
                    partnerId = network,
                    adapter = factory(context)
                )
            }
        }
    }

    /**
     * Allow host apps or tests to register custom runtime adapter factories before initialize().
     */
    fun registerRuntimeAdapterFactory(network: String, factory: (Context) -> AdNetworkAdapterV2) {
        runtimeFactories[network.lowercase()] = factory
    }

    /**
     * Register an already-constructed adapter instance under a network key.
     */
    fun register(network: String, adapter: AdAdapter) {
        adapters[network] = adapter
    }

    /**
     * Retrieve an adapter by network key.
     */
    fun getAdapter(network: String): AdAdapter? = adapters[network]

    /**
     * List registered networks.
     */
    fun getRegisteredNetworks(): List<String> {
        val legacy = adapters.keys().toList()
        val runtime = runtimeAdapters.keys.toList()
        return (legacy + runtime).distinct()
    }

    /**
     * Runtime adapter helpers used by MediationSDK when routing through the V2 pipeline.
     */
    fun getRuntimeAdapters(networks: List<String>): List<RuntimeAdapterEntry> {
        if (networks.isEmpty()) return emptyList()
        return networks.mapNotNull { runtimeAdapters[it] }
    }

    fun getRuntimeEntry(network: String): RuntimeAdapterEntry? = runtimeAdapters[network]

    /**
     * Shutdown and cleanup all registered adapters.
     */
    fun shutdown() {
        adapters.values.forEach { a ->
            try { a.destroy() } catch (_: Throwable) {}
        }
        adapters.clear()
        runtimeAdapters.values.forEach { entry ->
            try { entry.shutdown() } catch (_: Throwable) {}
        }
        runtimeAdapters.clear()
    }

    class RuntimeAdapterEntry internal constructor(
        val partnerId: String,
        private val adapter: AdNetworkAdapterV2,
        private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    ) {
        private val runtime = AdapterRuntimeWrapper(adapter, partnerId, scope)
        private val initLock = Any()
        @Volatile private var lastInitSignature: Int? = null
        @Volatile private var initialized: Boolean = false

        fun ensureInitialized(config: com.rivalapexmediation.sdk.contract.AdapterConfig, timeoutMs: Int): InitResult {
            val signature = config.hashCode()
            if (initialized && lastInitSignature == signature) {
                return InitResult(success = true)
            }
            return synchronized(initLock) {
                if (initialized && lastInitSignature == signature) {
                    InitResult(success = true)
                } else {
                    val result = adapter.init(config, timeoutMs)
                    if (result.success) {
                        initialized = true
                        lastInitSignature = signature
                    } else {
                        initialized = false
                        lastInitSignature = null
                    }
                    result
                }
            }
        }

        suspend fun loadInterstitial(placement: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
            return runtime.loadInterstitialWithEnforcement(placement, meta, timeoutMs)
        }

        fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
            runtime.showInterstitialOnMain(handle, viewContext, callbacks)
        }

        fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
            runtime.showRewardedOnMain(handle, viewContext, callbacks)
        }

        fun invalidate(handle: AdHandle) {
            try {
                adapter.invalidate(handle)
            } catch (_: Throwable) { /* ignore */ }
        }

        fun shutdown() {
            scope.cancel()
        }
    }
}
