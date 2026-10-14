package com.rivalapexmediation.ctv.network

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import okhttp3.ConnectionPool
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
 * for Android TV to reduce cold-start latency for auction requests.
 *
 * Android TV typically runs on more powerful hardware than mobile,
 * so we can use larger connection pools.
 */
object NetworkWarmer {
    
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val isWarmedUp = AtomicBoolean(false)
    private val dnsCache = ConcurrentHashMap<String, List<InetAddress>>()
    private val warmedEndpoints = mutableSetOf<String>()
    
    // Android TV typically has 4+ cores, use larger pool
    private val poolSize = Runtime.getRuntime().availableProcessors().coerceIn(4, 8)
    
    private val sharedClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectionPool(ConnectionPool(poolSize, 5, TimeUnit.MINUTES))
            .connectTimeout(2, TimeUnit.SECONDS)
            .readTimeout(5, TimeUnit.SECONDS)
            .writeTimeout(5, TimeUnit.SECONDS)
            .callTimeout(6, TimeUnit.SECONDS)
            .retryOnConnectionFailure(false)
            .followRedirects(false)
            .followSslRedirects(false)
            .protocols(listOf(Protocol.HTTP_2, Protocol.HTTP_1_1))
            .dns(object : okhttp3.Dns {
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
     */
    fun warmup(endpoint: String) {
        if (warmedEndpoints.contains(endpoint)) return
        
        scope.launch {
            try {
                val url = URL(endpoint)
                val hostname = url.host
                
                // DNS prefetch
                prefetchDns(hostname)
                
                // HTTP preconnect
                preconnect(endpoint)
                
                warmedEndpoints.add(endpoint)
                isWarmedUp.set(true)
            } catch (e: Exception) {
                // Best-effort
            }
        }
    }
    
    private fun prefetchDns(hostname: String) {
        if (dnsCache.containsKey(hostname)) return
        
        try {
            val addresses = InetAddress.getAllByName(hostname).toList()
            if (addresses.isNotEmpty()) {
                dnsCache[hostname] = addresses
            }
        } catch (e: Exception) {
            // Will retry on actual request
        }
    }
    
    private fun preconnect(endpoint: String) {
        try {
            val request = Request.Builder()
                .url("$endpoint/health")
                .head()
                .build()
            
            sharedClient.newBuilder()
                .callTimeout(2, TimeUnit.SECONDS)
                .build()
                .newCall(request)
                .execute()
                .close()
        } catch (e: Exception) {
            // Best-effort
        }
    }
    
    fun getWarmedClient(): OkHttpClient = sharedClient
    
    fun isWarmedUp(): Boolean = isWarmedUp.get()
    
    fun getCachedDns(hostname: String): List<InetAddress>? = dnsCache[hostname]
    
    fun reset() {
        dnsCache.clear()
        warmedEndpoints.clear()
        isWarmedUp.set(false)
    }
    
    fun getDiagnostics(): Map<String, Any> = mapOf(
        "isWarmedUp" to isWarmedUp.get(),
        "warmedEndpoints" to warmedEndpoints.toList(),
        "dnsCacheSize" to dnsCache.size,
        "cachedHosts" to dnsCache.keys.toList(),
        "connectionPoolSize" to poolSize
    )
}
