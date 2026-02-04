package com.rivalapexmediation.sdk.adapter

import android.content.Context
import com.rivalapexmediation.sdk.models.AdAdapter
import com.rivalapexmediation.sdk.models.ValidationResult
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicReference

/**
 * Adapter Health Status API
 * 
 * Tracks initialization status and health metrics for each ad network adapter.
 * Industry standard alignment with AppLovin MAX's InitializationStatus enum.
 */
object AdapterHealthManager {
    
    /**
     * Initialization status enum matching AppLovin MAX pattern
     */
    enum class InitializationStatus {
        /** Adapter has not been initialized yet */
        NOT_INITIALIZED,
        
        /** Adapter is currently initializing */
        INITIALIZING,
        
        /** Adapter initialized successfully */
        INITIALIZED_SUCCESS,
        
        /** Adapter failed to initialize */
        INITIALIZED_FAILURE,
        
        /** Adapter initialization status unknown (e.g., no callback from SDK) */
        INITIALIZED_UNKNOWN,
        
        /** Adapter does not require initialization */
        DOES_NOT_APPLY
    }
    
    /**
     * Health status for an individual adapter
     */
    data class AdapterHealth(
        val networkName: String,
        val status: InitializationStatus,
        val sdkVersion: String?,
        val adapterVersion: String?,
        val lastStatusChange: Long = System.currentTimeMillis(),
        val errorMessage: String? = null,
        val errorCode: String? = null,
        val capabilities: AdapterCapabilities = AdapterCapabilities(),
        val metrics: AdapterMetrics = AdapterMetrics()
    ) {
        fun isReady(): Boolean = status == InitializationStatus.INITIALIZED_SUCCESS || 
                                  status == InitializationStatus.DOES_NOT_APPLY
        
        fun toMap(): Map<String, Any?> = mapOf(
            "networkName" to networkName,
            "status" to status.name,
            "sdkVersion" to sdkVersion,
            "adapterVersion" to adapterVersion,
            "lastStatusChange" to lastStatusChange,
            "errorMessage" to errorMessage,
            "errorCode" to errorCode,
            "isReady" to isReady(),
            "capabilities" to capabilities.toMap(),
            "metrics" to metrics.toMap()
        )
    }
    
    /**
     * Capabilities supported by an adapter
     */
    data class AdapterCapabilities(
        val supportsBanner: Boolean = false,
        val supportsInterstitial: Boolean = false,
        val supportsRewarded: Boolean = false,
        val supportsRewardedInterstitial: Boolean = false,
        val supportsNative: Boolean = false,
        val supportsAppOpen: Boolean = false,
        val supportsSignalCollection: Boolean = false,
        val supportsS2SBidding: Boolean = false
    ) {
        fun toMap(): Map<String, Boolean> = mapOf(
            "banner" to supportsBanner,
            "interstitial" to supportsInterstitial,
            "rewarded" to supportsRewarded,
            "rewardedInterstitial" to supportsRewardedInterstitial,
            "native" to supportsNative,
            "appOpen" to supportsAppOpen,
            "signalCollection" to supportsSignalCollection,
            "s2sBidding" to supportsS2SBidding
        )
    }
    
    /**
     * Runtime metrics for an adapter
     */
    data class AdapterMetrics(
        val totalRequests: Long = 0,
        val successfulFills: Long = 0,
        val noFills: Long = 0,
        val errors: Long = 0,
        val timeouts: Long = 0,
        val avgLoadTimeMs: Double = 0.0,
        val lastRequestTime: Long? = null,
        val fillRate: Double = if (totalRequests > 0) (successfulFills.toDouble() / totalRequests) * 100 else 0.0
    ) {
        fun toMap(): Map<String, Any?> = mapOf(
            "totalRequests" to totalRequests,
            "successfulFills" to successfulFills,
            "noFills" to noFills,
            "errors" to errors,
            "timeouts" to timeouts,
            "avgLoadTimeMs" to avgLoadTimeMs,
            "lastRequestTime" to lastRequestTime,
            "fillRate" to fillRate
        )
        
        fun withRequest(success: Boolean, loadTimeMs: Long): AdapterMetrics {
            val newTotal = totalRequests + 1
            val newFills = if (success) successfulFills + 1 else successfulFills
            val newNoFills = if (!success) noFills + 1 else noFills
            val newAvgTime = ((avgLoadTimeMs * totalRequests) + loadTimeMs) / newTotal
            return copy(
                totalRequests = newTotal,
                successfulFills = newFills,
                noFills = newNoFills,
                avgLoadTimeMs = newAvgTime,
                lastRequestTime = System.currentTimeMillis()
            )
        }
        
        fun withError(): AdapterMetrics = copy(
            errors = errors + 1,
            lastRequestTime = System.currentTimeMillis()
        )
        
        fun withTimeout(): AdapterMetrics = copy(
            timeouts = timeouts + 1,
            lastRequestTime = System.currentTimeMillis()
        )
    }
    
    /**
     * Listener for adapter status changes
     */
    interface AdapterStatusListener {
        fun onAdapterStatusChanged(networkName: String, health: AdapterHealth)
    }
    
    // Internal storage
    private val healthStatus = ConcurrentHashMap<String, AtomicReference<AdapterHealth>>()
    private val statusListeners = mutableListOf<AdapterStatusListener>()
    private val metricsLock = Any()
    
    /**
     * Get current health status for an adapter
     */
    @JvmStatic
    fun getAdapterHealth(networkName: String): AdapterHealth? {
        return healthStatus[networkName.lowercase()]?.get()
    }
    
    /**
     * Get health status for all adapters
     */
    @JvmStatic
    fun getAllAdapterHealth(): Map<String, AdapterHealth> {
        return healthStatus.mapValues { it.value.get() }
    }
    
    /**
     * Get list of adapters that are ready
     */
    @JvmStatic
    fun getReadyAdapters(): List<String> {
        return healthStatus.filter { it.value.get().isReady() }.keys.toList()
    }
    
    /**
     * Get list of adapters that failed initialization
     */
    @JvmStatic
    fun getFailedAdapters(): List<AdapterHealth> {
        return healthStatus.values.map { it.get() }
            .filter { it.status == InitializationStatus.INITIALIZED_FAILURE }
    }
    
    /**
     * Register a listener for status changes
     */
    @JvmStatic
    fun addStatusListener(listener: AdapterStatusListener) {
        synchronized(statusListeners) {
            statusListeners.add(listener)
        }
    }
    
    /**
     * Remove a status listener
     */
    @JvmStatic
    fun removeStatusListener(listener: AdapterStatusListener) {
        synchronized(statusListeners) {
            statusListeners.remove(listener)
        }
    }
    
    // Internal methods for SDK to update status
    
    internal fun registerAdapter(
        networkName: String,
        adapterVersion: String?,
        capabilities: AdapterCapabilities = AdapterCapabilities()
    ) {
        val key = networkName.lowercase()
        healthStatus.putIfAbsent(key, AtomicReference(
            AdapterHealth(
                networkName = networkName,
                status = InitializationStatus.NOT_INITIALIZED,
                sdkVersion = null,
                adapterVersion = adapterVersion,
                capabilities = capabilities
            )
        ))
    }
    
    internal fun updateStatus(
        networkName: String,
        status: InitializationStatus,
        sdkVersion: String? = null,
        errorMessage: String? = null,
        errorCode: String? = null
    ) {
        val key = networkName.lowercase()
        val ref = healthStatus.getOrPut(key) {
            AtomicReference(AdapterHealth(
                networkName = networkName,
                status = status,
                sdkVersion = sdkVersion,
                adapterVersion = null
            ))
        }
        
        val current = ref.get()
        val updated = current.copy(
            status = status,
            sdkVersion = sdkVersion ?: current.sdkVersion,
            lastStatusChange = System.currentTimeMillis(),
            errorMessage = errorMessage,
            errorCode = errorCode
        )
        ref.set(updated)
        
        notifyListeners(networkName, updated)
    }
    
    internal fun recordRequest(networkName: String, success: Boolean, loadTimeMs: Long) {
        val key = networkName.lowercase()
        val ref = healthStatus[key] ?: return
        
        synchronized(metricsLock) {
            val current = ref.get()
            val updatedMetrics = current.metrics.withRequest(success, loadTimeMs)
            ref.set(current.copy(metrics = updatedMetrics))
        }
    }
    
    internal fun recordError(networkName: String) {
        val key = networkName.lowercase()
        val ref = healthStatus[key] ?: return
        
        synchronized(metricsLock) {
            val current = ref.get()
            ref.set(current.copy(metrics = current.metrics.withError()))
        }
    }
    
    internal fun recordTimeout(networkName: String) {
        val key = networkName.lowercase()
        val ref = healthStatus[key] ?: return
        
        synchronized(metricsLock) {
            val current = ref.get()
            ref.set(current.copy(metrics = current.metrics.withTimeout()))
        }
    }
    
    private fun notifyListeners(networkName: String, health: AdapterHealth) {
        val listeners = synchronized(statusListeners) { statusListeners.toList() }
        listeners.forEach { listener ->
            try {
                listener.onAdapterStatusChanged(networkName, health)
            } catch (e: Exception) {
                // Ignore listener exceptions
            }
        }
    }
    
    /**
     * Reset all health data (primarily for testing)
     */
    internal fun reset() {
        healthStatus.clear()
    }
}

/**
 * Extension interface for adapters that support health reporting
 */
interface HealthReportingAdapter : AdAdapter {
    fun getSdkVersion(): String
    fun getAdapterVersion(): String
    fun getCapabilities(): AdapterHealthManager.AdapterCapabilities
    fun getInitializationStatus(): AdapterHealthManager.InitializationStatus
}
