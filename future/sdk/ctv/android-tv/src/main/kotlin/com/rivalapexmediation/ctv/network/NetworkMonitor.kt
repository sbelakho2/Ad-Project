package com.rivalapexmediation.ctv.network

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.Handler
import android.os.Looper
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * NetworkMonitor for Android TV - Monitors network connectivity for CTV applications.
 *
 * This class provides:
 * - Real-time network state monitoring using ConnectivityManager.NetworkCallback
 * - Fast-fail for no-network conditions (critical for CTV ad breaks)
 * - Connection quality assessment optimized for streaming
 * - Callback-based network state change notifications
 */
object NetworkMonitor {
    
    /** Fast-fail timeout when offline (ms) - shorter for CTV ad breaks */
    const val OFFLINE_FAST_FAIL_TIMEOUT_MS = 50L
    
    /** Normal network timeout (ms) */
    const val NORMAL_TIMEOUT_MS = 8000L
    
    /** Network state information */
    data class NetworkState(
        val isConnected: Boolean,
        val connectionType: ConnectionType,
        val hasInternet: Boolean,
        val bandwidthKbps: Int,
        val timestamp: Long = System.currentTimeMillis()
    ) {
        companion object {
            val OFFLINE = NetworkState(
                isConnected = false,
                connectionType = ConnectionType.NONE,
                hasInternet = false,
                bandwidthKbps = 0
            )
        }
    }
    
    /** Connection type enumeration - Android TV typically uses ethernet */
    enum class ConnectionType {
        NONE,
        ETHERNET,
        WIFI,
        OTHER
    }
    
    /** Result of a pre-flight check before network operations */
    sealed class PreflightResult {
        /** Network is available, proceed with operation */
        data class Proceed(val state: NetworkState) : PreflightResult()
        
        /** Network unavailable, fast-fail with this error */
        data class FastFail(val reason: String, val state: NetworkState) : PreflightResult()
    }
    
    /** Listener interface for network state changes */
    fun interface NetworkStateListener {
        fun onNetworkStateChanged(state: NetworkState)
    }
    
    private var context: Context? = null
    private var connectivityManager: ConnectivityManager? = null
    
    private val currentState = AtomicReference(NetworkState.OFFLINE)
    private val isMonitoring = AtomicBoolean(false)
    private val listeners = ConcurrentHashMap<String, NetworkStateListener>()
    
    private var lastConnectivityChangeTime = 0L
    
    private val mainHandler = Handler(Looper.getMainLooper())
    
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            updateNetworkState()
        }
        
        override fun onLost(network: Network) {
            lastConnectivityChangeTime = System.currentTimeMillis()
            currentState.set(NetworkState.OFFLINE)
            notifyListeners()
        }
        
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            updateNetworkState()
        }
    }
    
    /**
     * Initializes the NetworkMonitor with application context.
     * Should be called early in app lifecycle.
     */
    fun init(context: Context) {
        this.context = context.applicationContext
        this.connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        
        // Get initial state
        updateNetworkState()
    }
    
    /**
     * Starts monitoring network state changes.
     */
    fun startMonitoring() {
        if (isMonitoring.getAndSet(true)) return
        
        val cm = connectivityManager ?: return
        
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        
        try {
            cm.registerNetworkCallback(request, networkCallback)
        } catch (e: Exception) {
            isMonitoring.set(false)
        }
    }
    
    /**
     * Stops monitoring network state changes.
     */
    fun stopMonitoring() {
        if (!isMonitoring.getAndSet(false)) return
        
        try {
            connectivityManager?.unregisterNetworkCallback(networkCallback)
        } catch (e: Exception) {
            // Ignore if not registered
        }
    }
    
    /**
     * Gets the current network state.
     */
    val state: NetworkState
        get() = currentState.get()
    
    /**
     * Checks if network is currently connected.
     */
    val isConnected: Boolean
        get() = currentState.get().isConnected
    
    /**
     * Performs a preflight check before network operations.
     * Returns immediately if offline, allowing fast-fail behavior.
     */
    fun preflight(): PreflightResult {
        val state = currentState.get()
        
        return if (state.isConnected && state.hasInternet) {
            PreflightResult.Proceed(state)
        } else {
            PreflightResult.FastFail("No network connection - ad break unavailable", state)
        }
    }
    
    /**
     * Gets the appropriate timeout based on network state.
     * Returns fast-fail timeout when offline.
     */
    fun effectiveTimeoutMs(): Long {
        return if (isConnected) NORMAL_TIMEOUT_MS else OFFLINE_FAST_FAIL_TIMEOUT_MS
    }
    
    /**
     * Adds a listener for network state changes.
     * @return A token to use when removing the listener
     */
    fun addListener(listener: NetworkStateListener): String {
        val token = java.util.UUID.randomUUID().toString()
        listeners[token] = listener
        
        // Immediately notify with current state
        listener.onNetworkStateChanged(currentState.get())
        return token
    }
    
    /**
     * Removes a listener using its token.
     */
    fun removeListener(token: String) {
        listeners.remove(token)
    }
    
    /**
     * Checks if connection is suitable for high-bitrate streaming.
     * Ethernet is preferred for CTV.
     */
    fun isSuitableForStreaming(): Boolean {
        val state = currentState.get()
        if (!state.isConnected || !state.hasInternet) return false
        
        // Ethernet is always preferred for CTV
        if (state.connectionType == ConnectionType.ETHERNET) {
            return true
        }
        
        // WiFi with good bandwidth is acceptable
        // 5000 kbps is reasonable for 1080p streaming
        return state.connectionType == ConnectionType.WIFI && state.bandwidthKbps >= 5000
    }
    
    /**
     * Gets streaming quality hints for adaptive bitrate.
     */
    fun streamingQualityHints(): Map<String, Any> {
        val state = currentState.get()
        return mapOf(
            "isConnected" to state.isConnected,
            "connectionType" to state.connectionType.name,
            "bandwidthKbps" to state.bandwidthKbps,
            "suggestedQuality" to if (isSuitableForStreaming()) "high" else "adaptive",
            "shouldBuffer" to !isSuitableForStreaming(),
            "fastFailTimeoutMs" to OFFLINE_FAST_FAIL_TIMEOUT_MS,
            "normalTimeoutMs" to NORMAL_TIMEOUT_MS
        )
    }
    
    /**
     * Gets time since last connectivity change (for rate limiting).
     */
    fun timeSinceLastChangeMs(): Long {
        return if (lastConnectivityChangeTime == 0L) {
            -1L
        } else {
            System.currentTimeMillis() - lastConnectivityChangeTime
        }
    }
    
    private fun updateNetworkState() {
        val cm = connectivityManager ?: return
        
        val activeNetwork = cm.activeNetwork
        val capabilities = activeNetwork?.let { cm.getNetworkCapabilities(it) }
        
        val connectionType = when {
            capabilities == null -> ConnectionType.NONE
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> ConnectionType.ETHERNET
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> ConnectionType.WIFI
            else -> ConnectionType.OTHER
        }
        
        val hasInternet = capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true &&
                capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        
        val bandwidthKbps = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            capabilities?.linkDownstreamBandwidthKbps ?: 0
        } else {
            0
        }
        
        val newState = NetworkState(
            isConnected = activeNetwork != null && capabilities != null,
            connectionType = connectionType,
            hasInternet = hasInternet,
            bandwidthKbps = bandwidthKbps
        )
        
        val oldState = currentState.getAndSet(newState)
        
        if (oldState != newState) {
            lastConnectivityChangeTime = System.currentTimeMillis()
            notifyListeners()
        }
    }
    
    private fun notifyListeners() {
        val state = currentState.get()
        for ((_, listener) in listeners) {
            mainHandler.post {
                listener.onNetworkStateChanged(state)
            }
        }
    }
}

/**
 * Exception for fast-fail on no-network conditions.
 */
class NoNetworkException(
    message: String,
    val networkState: NetworkMonitor.NetworkState
) : Exception(message)
