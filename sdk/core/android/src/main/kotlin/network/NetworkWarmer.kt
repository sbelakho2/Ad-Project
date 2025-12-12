package com.rivalapexmediation.sdk.network

import android.os.Build
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import okhttp3.ConnectionPool
import okhttp3.Dns
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import java.net.InetAddress
import java.net.URL
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/**
 * NetworkWarmer provides HTTP preconnect and DNS prefetch functionality
 * to reduce cold-start latency for auction requests.
 * 
 * Key features:
 * - DNS prefetch on SDK init (caches DNS resolution)
 * - HTTP/2 connection preconnect (establishes TLS + ALPN early)
 * - Connection pooling warmup
 * - Thread-safe singleton pattern
 * 
 * Usage:
 * ```kotlin
 * // At SDK init time:
 * NetworkWarmer.warmup("https://api.apexmediation.ee")
 * 
 * // Get pre-warmed client for requests:
 * val client = NetworkWarmer.getWarmedClient()
 * ```
 */
object NetworkWarmer {
    
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val isWarmedUp = AtomicBoolean(false)
    private val dnsCache = ConcurrentHashMap<String, List<InetAddress>>()
    private var warmedEndpoints = mutableSetOf<String>()
    
    // Shared client with optimal connection pooling
    private val sharedClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectionPool(ConnectionPool(
                maxIdleConnections = Runtime.getRuntime().availableProcessors().coerceIn(2, 8),
                keepAliveDuration = 5,
                timeUnit = TimeUnit.MINUTES
            ))
            .connectTimeout(2, TimeUnit.SECONDS)
            .readTimeout(5, TimeUnit.SECONDS)
            .writeTimeout(5, TimeUnit.SECONDS)
            .callTimeout(6, TimeUnit.SECONDS)
            .retryOnConnectionFailure(false)
            .followRedirects(false)
            .followSslRedirects(false)
            .protocols(listOf(Protocol.HTTP_2, Protocol.HTTP_1_1))
            // DNS cache integration
            .dns(object : Dns {
                override fun lookup(hostname: String): List<InetAddress> {
                    return dnsCache[hostname] ?: InetAddress.getAllByName(hostname).toList().also {
                        dnsCache[hostname] = it
                    }
                }
            })
            .build()
    }
    
    /**
     * Warm up connections to the specified endpoint.
     * This performs:
     * 1. DNS prefetch (resolves hostname to IP addresses)
     * 2. HTTP preconnect (establishes TCP + TLS connection)
     * 
     * Call this at SDK initialization time for best cold-start performance.
     * 
     * @param endpoint Base URL of the auction endpoint (e.g., "https://api.apexmediation.ee")
     */
    fun warmup(endpoint: String) {
        if (warmedEndpoints.contains(endpoint)) return
        
        scope.launch {
            try {
                val url = URL(endpoint)
                val hostname = url.host
                
                // Step 1: DNS prefetch
                prefetchDns(hostname)
                
                // Step 2: HTTP preconnect (HEAD request to warm connection pool)
                preconnect(endpoint)
                
                warmedEndpoints.add(endpoint)
                isWarmedUp.set(true)
            } catch (e: Exception) {
                // Warmup is best-effort; don't crash on failure
            }
        }
    }
    
    /**
     * Prefetch DNS for a hostname, caching the result.
     */
    private suspend fun prefetchDns(hostname: String) {
        if (dnsCache.containsKey(hostname)) return
        
        try {
            val addresses = InetAddress.getAllByName(hostname).toList()
            if (addresses.isNotEmpty()) {
                dnsCache[hostname] = addresses
            }
        } catch (e: Exception) {
            // DNS resolution failed; will retry on actual request
        }
    }
    
    /**
     * Establish a connection to the endpoint using a lightweight HEAD request.
     * This warms the connection pool for subsequent requests.
     */
    private fun preconnect(endpoint: String) {
        try {
            val request = Request.Builder()
                .url("$endpoint/health")
                .head()
                .build()
            
            // Execute synchronously but with short timeout
            sharedClient.newBuilder()
                .callTimeout(2, TimeUnit.SECONDS)
                .build()
                .newCall(request)
                .execute()
                .close()
        } catch (e: Exception) {
            // Preconnect is best-effort
        }
    }
    
    /**
     * Get the pre-warmed OkHttpClient instance.
     * This client has DNS caching and connection pooling already configured.
     */
    fun getWarmedClient(): OkHttpClient = sharedClient
    
    /**
     * Check if warmup has completed for at least one endpoint.
     */
    fun isWarmedUp(): Boolean = isWarmedUp.get()
    
    /**
     * Get cached DNS entries for a hostname, if available.
     */
    fun getCachedDns(hostname: String): List<InetAddress>? = dnsCache[hostname]
    
    /**
     * Clear all cached DNS entries and warmed endpoints.
     * Primarily for testing.
     */
    fun reset() {
        dnsCache.clear()
        warmedEndpoints.clear()
        isWarmedUp.set(false)
    }
    
    /**
     * Get diagnostics about current warmup state.
     */
    fun getDiagnostics(): Map<String, Any> = mapOf(
        "isWarmedUp" to isWarmedUp.get(),
        "warmedEndpoints" to warmedEndpoints.toList(),
        "dnsCacheSize" to dnsCache.size,
        "cachedHosts" to dnsCache.keys.toList(),
        "connectionPoolSize" to Runtime.getRuntime().availableProcessors().coerceIn(2, 8),
        "sdkVersion" to (try { com.rivalapexmediation.sdk.BuildConfig.SDK_VERSION } catch (_: Throwable) { "unknown" })
    )
}
