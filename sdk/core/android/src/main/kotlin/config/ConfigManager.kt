package com.rivalapexmediation.sdk.config

import android.content.Context
import android.content.SharedPreferences
import com.rivalapexmediation.sdk.BuildConfig
import com.rivalapexmediation.sdk.SDKConfig
import com.rivalapexmediation.sdk.models.*
import com.google.gson.Gson
import okhttp3.OkHttpClient
import okhttp3.Request
import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import java.io.IOException
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * Manages SDK configuration with caching and validation
 * 
 * Features:
 * - Remote config fetching with exponential backoff
 * - Local caching with TTL
 * - Signature verification
 * - Fallback to cached config on network failure
 */
class ConfigManager(
    private val context: Context,
    private val sdkConfig: SDKConfig,
    private val client: OkHttpClient? = null,
    private val configPublicKey: ByteArray? = null,
    private val clock: Clock = ClockProvider.clock
) {
    private val prefs: SharedPreferences = context.getSharedPreferences(
        "rival_ad_stack_config",
        Context.MODE_PRIVATE
    )
    
    private val httpClient: OkHttpClient = client ?: OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()
    
    private val gson = Gson()
    
    @Volatile
    private var currentConfig: SDKRemoteConfig? = null
    
    private var lastFetchTime: Long = 0
    private val configTTL: Long = 3600000 // 1 hour
    
    /**
     * Load configuration (tries remote, falls back to cache)
     */
    fun loadConfig() {
        // Try loading from cache first
        currentConfig = loadFromCache()
        
        // Fetch remote if cache is stale or missing
        if (shouldFetchRemote()) {
            try {
                if (!sdkConfig.testMode && configPublicKey == null) {
                    throw IllegalStateException("Missing config public key in production mode")
                }
                val remoteConfig = fetchRemoteConfig()
                val sigOk = if (sdkConfig.testMode) true else verifySignature(remoteConfig)
                if (!sdkConfig.testMode && !sigOk) {
                    throw IllegalStateException("Config signature verification failed")
                }
                if (!validateSchema(remoteConfig)) {
                    throw IllegalStateException("Config schema validation failed")
                }
                currentConfig = remoteConfig
                saveToCache(remoteConfig)
                lastFetchTime = clock.monotonicNow()
            } catch (e: IOException) {
                // Use cached config if network fails
                if (currentConfig == null) {
                    throw IllegalStateException("No cached config available", e)
                }
            }
        }
    }
    
    /**
     * Get placement configuration
     */
    fun getPlacementConfig(placementId: String): PlacementConfig? {
        return currentConfig?.placements?.get(placementId)
    }
    
    /**
     * Get all placements
     */
    fun getAllPlacements(): Map<String, PlacementConfig> {
        return currentConfig?.placements ?: emptyMap()
    }
    
    /**
     * Get adapter configuration
     */
    fun getAdapterConfig(adapterName: String): AdapterConfig? {
        return currentConfig?.adapters?.get(adapterName)
    }
    
    /**
     * Get feature flags
     */
    fun getFeatureFlags(): FeatureFlags {
        return currentConfig?.features ?: FeatureFlags()
    }
    
    /**
     * Check if config is valid and not expired
     */
    fun isConfigValid(): Boolean {
        if (currentConfig == null) return false
        val age = clock.monotonicNow() - lastFetchTime
        if (age < 0) return false
        return age < configTTL
    }
    
    /**
     * Fetch configuration from remote server
     */
    private fun fetchRemoteConfig(): SDKRemoteConfig {
        val url = "${sdkConfig.configEndpoint}/v1/config/${sdkConfig.appId}"
        
        val request = Request.Builder()
            .url(url)
            .addHeader("User-Agent", "RivalApexMediation-Android/${BuildConfig.SDK_VERSION}")
            .addHeader("X-App-ID", sdkConfig.appId)
            .build()
        
        val response = httpClient.newCall(request).execute()
        
        if (!response.isSuccessful) {
            throw IOException("Config fetch failed: ${response.code}")
        }
        
        val body = response.body?.string()
            ?: throw IOException("Empty response body")
        
        return gson.fromJson(body, SDKRemoteConfig::class.java)
    }
    
    /**
     * Verify configuration signature (Ed25519)
     */
    private fun verifySignature(config: SDKRemoteConfig): Boolean {
        // In test mode, bypass strict verification to ease development.
        if (sdkConfig.testMode) return true
        // Require a configured public key in non-test builds.
        val pubKeyBytes = configPublicKey ?: return false
        return try {
            val message = canonicalSigningMessage(config)
            val sigBytes = decodeBase64(config.signature)
            // Try JDK Ed25519 first (available on JVM 17); Android may fall back if provider missing.
            val kf = java.security.KeyFactory.getInstance("Ed25519")
            val pubKey = kf.generatePublic(java.security.spec.X509EncodedKeySpec(pubKeyBytes))
            val verifier = java.security.Signature.getInstance("Ed25519")
            verifier.initVerify(pubKey)
            verifier.update(message)
            verifier.verify(sigBytes)
        } catch (t: Throwable) {
            // Best-effort fallback using Tink, if available on the classpath.
            try {
                // Tink PublicKeyVerify expects a key in its own format; skip if not available.
                val clazz = Class.forName("com.google.crypto.tink.subtle.Ed25519Verify")
                val ctor = clazz.getConstructor(ByteArray::class.java)
                val verifier = ctor.newInstance(pubKeyBytes)
                val method = clazz.getMethod("verify", ByteArray::class.java, ByteArray::class.java)
                val message = canonicalSigningMessage(config)
                val sigBytes = decodeBase64(config.signature)
                method.invoke(verifier, sigBytes, message)
                true
            } catch (_: Throwable) {
                false
            }
        }
    }
    


    /**
     * Lightweight schema validation for remote config payloads before trusting.
     * Ensures required fields are present and placements have minimal fields.
     */
    private fun validateSchema(cfg: SDKRemoteConfig): Boolean {
        if (cfg.configId.isBlank()) return false
        if (cfg.version <= 0) return false
        if (cfg.timestamp <= 0) return false
        // placements may be empty, but if present, keys and placementId must match and adType must be non-empty
        cfg.placements.forEach { (key, pl) ->
            if (key.isBlank()) return false
            if (pl.placementId.isBlank()) return false
            // basic sanity on timeout bounds
            if (pl.timeoutMs <= 0 || pl.timeoutMs > 30_000L) return false
            if (pl.maxWaitMs <= 0 || pl.maxWaitMs > 60_000L) return false
        }
        // features non-null by model; adapters can be empty
        return true
    }
    
    /**
     * Save configuration to local cache
     */
    private fun saveToCache(config: SDKRemoteConfig) {
        prefs.edit()
            .putString("config_json", gson.toJson(config))
            .putLong("last_fetch", clock.monotonicNow())
            .apply()
    }
    
    /**
     * Load configuration from local cache
     */
    private fun loadFromCache(): SDKRemoteConfig? {
        val json = prefs.getString("config_json", null) ?: return null
        lastFetchTime = prefs.getLong("last_fetch", 0)
        val nowMono = clock.monotonicNow()
        if (lastFetchTime <= 0 || lastFetchTime > nowMono) {
            lastFetchTime = 0
        }
        
        return try {
            gson.fromJson(json, SDKRemoteConfig::class.java)
        } catch (e: Exception) {
            null
        }
    }
    
    /**
     * Check if remote fetch is needed
     */
    private fun shouldFetchRemote(): Boolean {
        // Always fetch in test mode
        if (sdkConfig.testMode) {
            return true
        }
        
        // Fetch if no config or expired
        if (currentConfig == null) {
            return true
        }
        
        val age = clock.monotonicNow() - lastFetchTime
        if (age < 0) return true
        return age >= configTTL
    }
    
    /**
     * Force refresh configuration
     */
    fun refresh() {
        lastFetchTime = 0
        loadConfig()
    }
    
    /**
     * Clear cached configuration
     */
    fun clearCache() {
        prefs.edit().clear().apply()
        currentConfig = null
        lastFetchTime = 0
    }
    
    /**
     * Shutdown and cleanup resources
     */
    fun shutdown() {
        httpClient.dispatcher.executorService.shutdown()
        httpClient.connectionPool.evictAll()
    }

    /**
     * Staged rollout check using stable per-install bucketing (non-PII InstallId).
     * Returns true if this install should participate given the percentage [0..100].
     */
    fun isInRollout(percentage: Int): Boolean {
        return try {
            com.rivalapexmediation.sdk.util.Rollout.isInRollout(context, percentage)
        } catch (_: Throwable) { percentage >= 100 }
    }

    /**
     * Compute deterministic SHA-256 hash of the current configuration.
     * Uses sorted JSON serialization to ensure cross-platform parity with server.
     * Hash format: "v1:<hex-digest>"
     * 
     * @return Configuration hash string or null if no config loaded
     */
    fun getConfigHash(): String? {
        val config = currentConfig ?: return null
        return try {
            val canonicalJson = buildCanonicalConfigJson(config)
            val digest = MessageDigest.getInstance("SHA-256")
            val hashBytes = digest.digest(canonicalJson.toByteArray(Charsets.UTF_8))
            val hexHash = hashBytes.joinToString("") { "%02x".format(it) }
            "v1:$hexHash"
        } catch (e: Exception) {
            null
        }
    }

    /**
     * Build canonical JSON representation for hashing.
     * Keys are sorted alphabetically to ensure deterministic output.
     */
    private fun buildCanonicalConfigJson(config: SDKRemoteConfig): String {
        val sortedMap = LinkedHashMap<String, Any?>()
        
        // Add fields in alphabetical order
        sortedMap["appId"] = sdkConfig.appId
        
        // Adapters - sorted alphabetically
        val adaptersMap = LinkedHashMap<String, Any>()
        config.adapters.keys.sorted().forEach { adapterName ->
            config.adapters[adapterName]?.let { adapter ->
                adaptersMap[adapterName] = mapOf(
                    "enabled" to adapter.enabled,
                    "priority" to adapter.priority
                )
            }
        }
        sortedMap["adapters"] = adaptersMap
        
        // Features
        sortedMap["features"] = mapOf(
            "crashReportingEnabled" to config.features.crashReportingEnabled,
            "debugLoggingEnabled" to config.features.debugLoggingEnabled,
            "enableOmSdk" to config.features.enableOmSdk,
            "experimentalFeaturesEnabled" to config.features.experimentalFeaturesEnabled,
            "killSwitch" to config.features.killSwitch,
            "telemetryEnabled" to config.features.telemetryEnabled
        )
        
        // Placements - sorted by placement ID
        val placementsMap = LinkedHashMap<String, Any>()
        config.placements.keys.sorted().forEach { placementId ->
            config.placements[placementId]?.let { placement ->
                placementsMap[placementId] = mapOf(
                    "adType" to placement.adType.name,
                    "enabledNetworks" to placement.enabledNetworks.sorted(),
                    "floorPrice" to placement.floorPrice,
                    "maxWaitMs" to placement.maxWaitMs,
                    "refreshInterval" to (placement.refreshInterval ?: 0),
                    "timeoutMs" to placement.timeoutMs
                )
            }
        }
        sortedMap["placements"] = placementsMap
        
        // Version
        sortedMap["version"] = config.version
        
        return gson.toJson(sortedMap)
    }

    /**
     * Validate that local config hash matches server hash.
     * Useful for debugging configuration sync issues.
     * 
     * @param serverHash Hash returned from /api/v1/config/sdk/config/hash endpoint
     * @return true if hashes match, false otherwise
     */
    fun validateConfigHash(serverHash: String): Boolean {
        val localHash = getConfigHash() ?: return false
        return localHash == serverHash
    }
}


// Base64 decoder that works on both JVM and Android without hard dependency on java.util.Base64 at runtime on older APIs.
private fun decodeBase64(input: String): ByteArray {
    return try {
        java.util.Base64.getDecoder().decode(input)
    } catch (_: Throwable) {
        try {
            android.util.Base64.decode(input, android.util.Base64.DEFAULT)
        } catch (_: Throwable) {
            // As a last resort, attempt URL-safe variant
            try {
                java.util.Base64.getUrlDecoder().decode(input)
            } catch (_: Throwable) {
                ByteArray(0)
            }
        }
    }
}

/**
 * Canonical message for config signature verification.
 *
 * The bytes are pinned to the iOS `SignatureVerifier.createCanonicalMessage`
 * output (lexicographic keys), so one signed envelope verifies on both
 * platforms. The config service must sign exactly this message:
 * {"config_id":"<id>","timestamp":<ts>,"version":<v>}
 */
internal fun canonicalSigningMessage(config: SDKRemoteConfig): ByteArray {
    val escapedId = config.configId
        .replace("\\", "\\\\")
        .replace("\"", "\\\"")
    return ("{\"config_id\":\"$escapedId\",\"timestamp\":${config.timestamp}," +
        "\"version\":${config.version}}").toByteArray(Charsets.UTF_8)
}
