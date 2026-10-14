package com.rivalapexmediation.sdk.adapter

import android.content.Context
import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.BuildConfig
import java.util.concurrent.ConcurrentHashMap

/**
 * Base adapter interface
 * 
 * All ad network adapters must implement this interface
 */
interface AdNetworkAdapter {
    /**
     * Adapter metadata
     */
    val networkName: String
    val version: String
    val minSDKVersion: String
    
    /**
     * Initialize adapter with app-level configuration
     */
    fun initialize(context: Context, config: Map<String, Any>)
    
    /**
     * Load an ad
     * 
     * @param placement Placement identifier
     * @param adType Type of ad to load
     * @param config Adapter-specific configuration
     * @param callback Callback for ad load result
     */
    fun loadAd(
        placement: String,
        adType: AdType,
        config: Map<String, Any>,
        callback: AdLoadCallback
    )
    
    /**
     * Check if adapter supports specific ad type
     */
    fun supportsAdType(adType: AdType): Boolean
    
    /**
     * Clean up adapter resources
     */
    fun destroy()
}

/**
 * Ad load callback
 */
interface AdLoadCallback {
    fun onAdLoaded(ad: Ad)
    fun onAdFailed(error: String, errorCode: Int)
}

/**
 * Adapter registry with dynamic loading
 * 
 * Features:
 * - Reflection-based adapter discovery
 * - Version compatibility checking
 * - Lazy initialization
 * - Thread-safe adapter management
 */
class AdapterRegistry(
    private val context: Context,
    private val sdkVersion: String
) {
    private val adapters = ConcurrentHashMap<String, AdapterWrapper>()
    private val adapterClasses = ConcurrentHashMap<String, Class<*>>()
    
    /**
     * Public diagnostics: how many adapters are registered (discovered or manually registered)
     */
    val registeredCount: Int get() = adapterClasses.size

    /** Return all registered adapter network names (sorted) */
    fun getAdapterNames(): List<String> = adapterClasses.keys().toList().sorted()
    
    /**
     * Adapter wrapper with metadata
     */
    private data class AdapterWrapper(
        val adapter: AdNetworkAdapter,
        val isInitialized: Boolean = false,
        val initError: String? = null
    )
    
    init {
        // Only attempt reflection-based discovery in debug or when explicitly enabled.
        if (isSandboxAdapterDiscoveryEnabled()) {
            discoverAdapters()
        }
    }
    
    /**
     * Discover available adapters via reflection
     */
    private fun discoverAdapters() {
        // Common adapter package names
        val adapterPackages = listOf(
            "com.rivalapexmediation.adapter.admob",
            "com.rivalapexmediation.adapter.applovin",
            "com.rivalapexmediation.adapter.ironsource",
            "com.rivalapexmediation.adapter.unity",
            "com.rivalapexmediation.adapter.facebook",
            "com.rivalapexmediation.adapter.vungle",
            "com.rivalapexmediation.adapter.chartboost",
            "com.rivalapexmediation.adapter.pangle",
            "com.rivalapexmediation.adapter.mintegral",
            "com.rivalapexmediation.adapter.adcolony",
            "com.rivalapexmediation.adapter.tapjoy",
            "com.rivalapexmediation.adapter.moloco",
            "com.rivalapexmediation.adapter.fyber",
            "com.rivalapexmediation.adapter.smaato",
            "com.rivalapexmediation.adapter.amazon"
        )

        for (packageName in adapterPackages) {
            try {
                val adapterClass = Class.forName("$packageName.Adapter")
                
                // Verify it implements AdNetworkAdapter
                if (AdNetworkAdapter::class.java.isAssignableFrom(adapterClass)) {
                    // Extract network name from package
                    val networkName = packageName.substringAfterLast(".")
                    adapterClasses[networkName] = adapterClass
                }
            } catch (e: ClassNotFoundException) {
                // Adapter not included in build
            }
        }
    }

    private fun isSandboxAdapterDiscoveryEnabled(): Boolean {
        // Enabled in debug builds or when a system property is set (e.g., in CI):
        // -DAPEX_SANDBOX_ADAPTERS=1 → java/system property "apex.sandbox.adapters=1"
        val prop = try {
            System.getProperty("apex.sandbox.adapters")
        } catch (_: Throwable) {
            null
        }
        return BuildConfig.DEBUG || prop == "1"
    }
    
    /**
     * Register custom adapter
     */
    fun registerAdapter(networkName: String, adapterClass: Class<out AdNetworkAdapter>) {
        adapterClasses[networkName] = adapterClass
    }
    
    /**
     * Get adapter instance (lazy initialization)
     */
    fun getAdapter(networkName: String): AdNetworkAdapter? {
        // Check if already initialized
        adapters[networkName]?.let { wrapper ->
            return if (wrapper.isInitialized) wrapper.adapter else null
        }
        
        // Load and initialize adapter
        val adapterClass = adapterClasses[networkName] ?: return null
        
        try {
            val adapter = adapterClass.newInstance() as AdNetworkAdapter
            
            // Check version compatibility
            if (!isCompatible(adapter.minSDKVersion)) {
                adapters[networkName] = AdapterWrapper(
                    adapter = adapter,
                    isInitialized = false,
                    initError = "Incompatible SDK version"
                )
                return null
            }
            
            adapters[networkName] = AdapterWrapper(
                adapter = adapter,
                isInitialized = false
            )
            
            return adapter
        } catch (e: Exception) {
            adapters[networkName] = AdapterWrapper(
                adapter = createDummyAdapter(networkName),
                isInitialized = false,
                initError = e.message
            )
            return null
        }
    }
    
    /**
     * Initialize adapter
     */
    fun initializeAdapter(networkName: String, config: Map<String, Any>) {
        val wrapper = adapters[networkName] ?: return
        
        if (wrapper.isInitialized) {
            return
        }
        
        try {
            wrapper.adapter.initialize(context, config)
            
            adapters[networkName] = wrapper.copy(
                isInitialized = true,
                initError = null
            )
        } catch (e: Exception) {
            adapters[networkName] = wrapper.copy(
                isInitialized = false,
                initError = e.message
            )
        }
    }
    
    /**
     * Check if adapter is initialized
     */
    fun isInitialized(networkName: String): Boolean {
        return adapters[networkName]?.isInitialized == true
    }
    
    /**
     * Get all available adapters
     */
    fun getAvailableAdapters(): List<String> {
        return adapterClasses.keys.toList()
    }
    
    /**
     * Get all initialized adapters
     */
    fun getInitializedAdapters(): List<String> {
        return adapters.filter { it.value.isInitialized }.keys.toList()
    }
    
    /**
     * Diagnostic report of adapter initialization status
     */
    data class InitializationStatus(
        val networkName: String,
        val registered: Boolean,
        val initialized: Boolean,
        val version: String?,
        val minSDKVersion: String?,
        val error: String?
    )

    fun getInitializationReport(): List<InitializationStatus> {
        val known = adapterClasses.keys
        val union = (known + adapters.keys).toSortedSet()
        return union.map { name ->
            val cls = adapterClasses[name]
            val wrapper = adapters[name]
            val adapter: AdNetworkAdapter? = when {
                wrapper != null -> wrapper.adapter
                cls != null -> try { (cls.newInstance() as AdNetworkAdapter) } catch (_: Throwable) { null }
                else -> null
            }
            InitializationStatus(
                networkName = name,
                registered = cls != null,
                initialized = wrapper?.isInitialized == true,
                version = adapter?.version,
                minSDKVersion = adapter?.minSDKVersion,
                error = wrapper?.initError
            )
        }
    }
    
    /**
     * Get adapter initialization error
     */
    fun getAdapterError(networkName: String): String? {
        return adapters[networkName]?.initError
    }
    
    /**
     * Check version compatibility
     */
    private fun isCompatible(minVersion: String): Boolean {
        return compareVersions(sdkVersion, minVersion) >= 0
    }
    
    /**
     * Compare semantic versions
     */
    private fun compareVersions(version1: String, version2: String): Int {
        val parts1 = version1.split(".").map { it.toIntOrNull() ?: 0 }
        val parts2 = version2.split(".").map { it.toIntOrNull() ?: 0 }
        
        for (i in 0 until maxOf(parts1.size, parts2.size)) {
            val part1 = parts1.getOrNull(i) ?: 0
            val part2 = parts2.getOrNull(i) ?: 0
            
            if (part1 != part2) {
                return part1.compareTo(part2)
            }
        }
        
        return 0
    }
    
    /**
     * Create dummy adapter for error handling
     */
    private fun createDummyAdapter(networkName: String): AdNetworkAdapter {
        return object : AdNetworkAdapter {
            override val networkName: String = networkName
            override val version: String = "0.0.0"
            override val minSDKVersion: String = "0.0.0"
            
            override fun initialize(context: Context, config: Map<String, Any>) {}
            
            override fun loadAd(
                placement: String,
                adType: AdType,
                config: Map<String, Any>,
                callback: AdLoadCallback
            ) {
                callback.onAdFailed("Adapter not available", -1)
            }
            
            override fun supportsAdType(adType: AdType): Boolean = false
            
            override fun destroy() {}
        }
    }
    
    /**
     * Clean up all adapters
     */
    fun destroy() {
        adapters.values.forEach { wrapper ->
            try {
                wrapper.adapter.destroy()
            } catch (e: Exception) {
                // Ignore cleanup errors
            }
        }
        
        adapters.clear()
        adapterClasses.clear()
    }
}

/**
 * Example adapter implementation (AdMob)
 */
class AdMobAdapter : AdNetworkAdapter {
    override val networkName = "admob"
    override val version = "1.0.0"
    override val minSDKVersion = "1.0.0"
    
    private var isInitialized = false
    
    override fun initialize(context: Context, config: Map<String, Any>) {
        val appId = config["app_id"] as? String ?: throw IllegalArgumentException("app_id required")
        
        // Initialize AdMob SDK
        // MobileAds.initialize(context, appId)
        
        isInitialized = true
    }
    
    override fun loadAd(
        placement: String,
        adType: AdType,
        config: Map<String, Any>,
        callback: AdLoadCallback
    ) {
        if (!isInitialized) {
            callback.onAdFailed("Adapter not initialized", -1)
            return
        }
        
        // Load ad from AdMob
        // Implementation depends on AdMob SDK integration
    }
    
    override fun supportsAdType(adType: AdType): Boolean {
        return when (adType) {
            AdType.BANNER, AdType.INTERSTITIAL, AdType.REWARDED -> true
            else -> false
        }
    }
    
    override fun destroy() {
        isInitialized = false
    }
}
