package com.rivalapexmediation.sdk.network

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * NetworkMonitor - Monitors network connectivity and provides fast-fail behavior.
 * 
 * This class provides:
 * - Real-time network state monitoring
 * - Fast-fail for no-network conditions (no UI jank)
 * - Connection quality assessment
 * - Callback-based network state change notifications
 */
class NetworkMonitor private constructor(context: Context) {
    
    companion object {
        @Volatile
        private var instance: NetworkMonitor? = null
        
        fun getInstance(context: Context): NetworkMonitor {
            return instance ?: synchronized(this) {
                instance ?: NetworkMonitor(context.applicationContext).also { instance = it }
            }
        }
        
        /** Fast-fail timeout when offline (milliseconds) */
        const val OFFLINE_FAST_FAIL_TIMEOUT_MS = 100L
        
        /** Normal network timeout (milliseconds) */
        const val NORMAL_TIMEOUT_MS = 10_000L
    }
    
    /**
     * Network state information.
     */
    data class NetworkState(
        val isConnected: Boolean,
        val connectionType: ConnectionType,
        val isMetered: Boolean,
        val hasInternetCapability: Boolean,
        val timestamp: Long
    ) {
        companion object {
            val OFFLINE = NetworkState(
                isConnected = false,
                connectionType = ConnectionType.NONE,
                isMetered = false,
                hasInternetCapability = false,
                timestamp = System.currentTimeMillis()
            )
        }
    }
    
    /**
     * Connection type enumeration.
     */
    enum class ConnectionType {
        NONE,
        WIFI,
        CELLULAR,
        ETHERNET,
        VPN,
        OTHER
    }
    
    /**
     * Result of a pre-flight check before network operations.
     */
    sealed class PreflightResult {
        /** Network is available, proceed with operation */
        data class Proceed(val state: NetworkState) : PreflightResult()
        
        /** Network unavailable, fast-fail with this error */
        data class FastFail(val reason: String, val state: NetworkState) : PreflightResult()
    }
    
    /**
     * Listener for network state changes.
     */
    interface NetworkStateListener {
        fun onNetworkStateChanged(state: NetworkState)
    }
    
    private val connectivityManager: ConnectivityManager =
        context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    
    private val currentState = AtomicReference(NetworkState.OFFLINE)
    private val isMonitoring = AtomicBoolean(false)
    private val listeners = CopyOnWriteArrayList<NetworkStateListener>()
    
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            updateNetworkState()
        }
        
        override fun onLost(network: Network) {
            updateNetworkState()
        }
        
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            updateNetworkState()
        }
    }
    
    init {
        // Get initial state
        updateNetworkState()
    }
    
    /**
     * Starts monitoring network state changes.
     */
    fun startMonitoring() {
        if (isMonitoring.compareAndSet(false, true)) {
            val request = NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .build()
            try {
                connectivityManager.registerNetworkCallback(request, networkCallback)
            } catch (e: Exception) {
                // May fail on some devices, fall back to polling
                isMonitoring.set(false)
            }
        }
    }
    
    /**
     * Stops monitoring network state changes.
     */
    fun stopMonitoring() {
        if (isMonitoring.compareAndSet(true, false)) {
            try {
                connectivityManager.unregisterNetworkCallback(networkCallback)
            } catch (e: Exception) {
                // Ignore unregister errors
            }
        }
    }
    
    /**
     * Gets the current network state.
     */
    fun getState(): NetworkState = currentState.get()
    
    /**
     * Checks if network is currently connected.
     */
    fun isConnected(): Boolean = currentState.get().isConnected
    
    /**
     * Checks if we're on a metered connection (cellular data).
     */
    fun isMetered(): Boolean = currentState.get().isMetered
    
    /**
     * Performs a preflight check before network operations.
     * Returns immediately if offline, allowing fast-fail behavior.
     */
    fun preflight(): PreflightResult {
        val state = updateNetworkState()
        
        return if (state.isConnected && state.hasInternetCapability) {
            PreflightResult.Proceed(state)
        } else {
            val reason = when {
                !state.isConnected -> "No network connection"
                !state.hasInternetCapability -> "No internet capability"
                else -> "Network unavailable"
            }
            PreflightResult.FastFail(reason, state)
        }
    }
    
    /**
     * Gets the appropriate timeout based on network state.
     * Returns fast-fail timeout when offline.
     */
    fun getEffectiveTimeout(): Long {
        return if (isConnected()) {
            NORMAL_TIMEOUT_MS
        } else {
            OFFLINE_FAST_FAIL_TIMEOUT_MS
        }
    }
    
    /**
     * Adds a listener for network state changes.
     */
    fun addListener(listener: NetworkStateListener) {
        listeners.add(listener)
        // Immediately notify with current state
        listener.onNetworkStateChanged(currentState.get())
    }
    
    /**
     * Removes a listener.
     */
    fun removeListener(listener: NetworkStateListener) {
        listeners.remove(listener)
    }
    
    /**
     * Gets network quality hints for adaptive behavior.
     */
    fun getQualityHints(): Map<String, Any> {
        val state = currentState.get()
        return mapOf(
            "isConnected" to state.isConnected,
            "connectionType" to state.connectionType.name,
            "isMetered" to state.isMetered,
            "suggestedTimeout" to getEffectiveTimeout(),
            "shouldReduceQuality" to (state.connectionType == ConnectionType.CELLULAR && state.isMetered)
        )
    }
    
    private fun updateNetworkState(): NetworkState {
        val network = connectivityManager.activeNetwork
        val capabilities = network?.let { connectivityManager.getNetworkCapabilities(it) }
        
        val newState = if (network != null && capabilities != null) {
            NetworkState(
                isConnected = true,
                connectionType = getConnectionType(capabilities),
                isMetered = !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED),
                hasInternetCapability = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                        capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED),
                timestamp = System.currentTimeMillis()
            )
        } else {
            NetworkState.OFFLINE.copy(timestamp = System.currentTimeMillis())
        }
        
        val previousState = currentState.getAndSet(newState)
        
        // Notify listeners if state changed
        if (previousState.isConnected != newState.isConnected ||
            previousState.connectionType != newState.connectionType) {
            listeners.forEach { it.onNetworkStateChanged(newState) }
        }
        
        return newState
    }
    
    private fun getConnectionType(capabilities: NetworkCapabilities): ConnectionType {
        return when {
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> ConnectionType.WIFI
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> ConnectionType.CELLULAR
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> ConnectionType.ETHERNET
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN) -> ConnectionType.VPN
            else -> ConnectionType.OTHER
        }
    }
}

/**
 * Exception thrown for fast-fail on no-network conditions.
 */
class NoNetworkException(
    message: String,
    val networkState: NetworkMonitor.NetworkState
) : Exception(message)
