package com.rivalapexmediation.ctv.ssai

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.media3.exoplayer.ExoPlayer
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

/**
 * SSAI Integration for Android TV SDK
 *
 * Main entry point for SSAI functionality. Provides a unified interface
 * for session management, manifest parsing, and player observation.
 */
class SSAIIntegration private constructor() {

    companion object {
        /** Shared instance for convenience */
        @Volatile
        private var instance: SSAIIntegration? = null

        @JvmStatic
        fun getInstance(): SSAIIntegration {
            return instance ?: synchronized(this) {
                instance ?: SSAIIntegration().also { instance = it }
            }
        }

        /**
         * Create a new SSAI integration instance (non-singleton)
         */
        @JvmStatic
        suspend fun create(
            config: SSAIConfig,
            clientConfig: SSAISessionClient.Config = SSAISessionClient.Config(),
            observerConfig: SSAIPlayerObserver.ObserverConfig = SSAIPlayerObserver.ObserverConfig()
        ): SSAIIntegration {
            val integration = SSAIIntegration()
            integration.initialize(config, clientConfig, observerConfig)
            return integration
        }
    }

    // Properties
    private var sessionClient: SSAISessionClient? = null
    private var playerObserver: SSAIPlayerObserver? = null
    private var manifestParser: SSAIManifestParser? = null

    private var currentSession: SSAISession? = null
    private var currentConfig: SSAIConfig? = null
    private var isInitialized = false

    private val mainHandler = Handler(Looper.getMainLooper())
    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())

    // Flows
    private val _adBreakEvents = MutableSharedFlow<SSAIAdBreak>()
    private val _adProgressEvents = MutableSharedFlow<Pair<SSAIAd, Double>>()
    private val _errorEvents = MutableSharedFlow<SSAIError>()

    /** Publisher for ad break events */
    val adBreakEvents: SharedFlow<SSAIAdBreak> = _adBreakEvents.asSharedFlow()

    /** Publisher for ad progress events */
    val adProgressEvents: SharedFlow<Pair<SSAIAd, Double>> = _adProgressEvents.asSharedFlow()

    /** Publisher for errors */
    val errorEvents: SharedFlow<SSAIError> = _errorEvents.asSharedFlow()

    // Public accessors
    val session: SSAISession? get() = currentSession
    val metrics: SSAIMetrics get() = sessionClient?.currentMetrics ?: SSAIMetrics()
    val isInAdBreak: Boolean get() = playerObserver?.isInAdBreak ?: false
    val currentAd: SSAIAd? get() = playerObserver?.currentAdInfo
    val currentAdBreak: SSAIAdBreak? get() = playerObserver?.currentAdBreakInfo
    val manifestUrl: String? get() = currentSession?.manifestUrl

    /**
     * Initialize SSAI integration with configuration
     */
    suspend fun initialize(
        config: SSAIConfig,
        clientConfig: SSAISessionClient.Config = SSAISessionClient.Config(),
        observerConfig: SSAIPlayerObserver.ObserverConfig = SSAIPlayerObserver.ObserverConfig()
    ): SSAISession {
        currentConfig = config

        // Create session client
        val client = SSAISessionClient(clientConfig)
        sessionClient = client

        // Create manifest parser
        manifestParser = SSAIManifestParser(config.debug)

        // Create session
        val session = client.createSession(config)
        currentSession = session

        // Create player observer
        val observer = SSAIPlayerObserver(client, observerConfig)
        playerObserver = observer

        // Forward observer events
        scope.launch {
            observer.adBreakEvents.collect { adBreak ->
                _adBreakEvents.emit(adBreak)
            }
        }

        scope.launch {
            observer.adProgressEvents.collect { (ad, progress) ->
                _adProgressEvents.emit(ad to progress)
            }
        }

        isInitialized = true

        if (config.debug) {
            println("[SSAI] Initialized with session: ${session.sessionId}")
        }

        return session
    }

    /**
     * Initialize with callback (for Java compatibility)
     */
    fun initialize(
        config: SSAIConfig,
        clientConfig: SSAISessionClient.Config = SSAISessionClient.Config(),
        observerConfig: SSAIPlayerObserver.ObserverConfig = SSAIPlayerObserver.ObserverConfig(),
        callback: (Result<SSAISession>) -> Unit
    ) {
        scope.launch {
            try {
                val session = initialize(config, clientConfig, observerConfig)
                mainHandler.post { callback(Result.success(session)) }
            } catch (e: Exception) {
                val error = when (e) {
                    is SSAIError -> e
                    else -> SSAIError("INIT_ERROR", e.message ?: "Unknown error", true)
                }
                mainHandler.post { callback(Result.failure(error)) }
            }
        }
    }

    /**
     * Attach to ExoPlayer for automatic tracking
     */
    fun attach(player: ExoPlayer) {
        val observer = playerObserver
        if (observer == null) {
            if (currentConfig?.debug == true) {
                println("[SSAI] Cannot attach - not initialized")
            }
            return
        }

        observer.attach(player)

        // Load ad breaks from session if available
        currentSession?.let { session ->
            observer.setAdBreaks(session.adBreaks)
        }
    }

    /**
     * Detach from player
     */
    fun detach() {
        playerObserver?.detach()
    }

    /**
     * Set observer listener for custom handling
     */
    fun setObserverListener(listener: SSAIPlayerObserver.Listener) {
        playerObserver?.listener = listener
    }

    /**
     * Track a custom event
     */
    fun trackEvent(event: SSAITrackingEvent) {
        sessionClient?.trackEvent(event)
    }

    /**
     * Skip current ad (for skip button)
     */
    fun skipCurrentAd() {
        playerObserver?.skipCurrentAd()
    }

    /**
     * Refresh session (for long-running streams)
     */
    suspend fun refreshSession(): SSAISession {
        val config = currentConfig
            ?: throw SSAIError("NOT_INITIALIZED", "SSAI not initialized", true)

        // End current session
        sessionClient?.endSession()

        // Create new session
        return initialize(config)
    }

    /**
     * Refresh session with callback
     */
    fun refreshSession(callback: (Result<SSAISession>) -> Unit) {
        scope.launch {
            try {
                val session = refreshSession()
                mainHandler.post { callback(Result.success(session)) }
            } catch (e: Exception) {
                val error = when (e) {
                    is SSAIError -> e
                    else -> SSAIError("REFRESH_ERROR", e.message ?: "Unknown error", true)
                }
                mainHandler.post { callback(Result.failure(error)) }
            }
        }
    }

    /**
     * End session and cleanup
     */
    fun shutdown(callback: ((Boolean) -> Unit)? = null) {
        playerObserver?.detach()
        sessionClient?.endSession { success ->
            sessionClient = null
            playerObserver = null
            manifestParser = null
            currentSession = null
            currentConfig = null
            isInitialized = false
            callback?.invoke(success)
        }
    }

    /**
     * Parse manifest for ad cues (for custom implementations)
     */
    suspend fun parseManifest(url: String): SSAIManifestParser.ParseResult {
        val parser = manifestParser
            ?: throw SSAIError("NOT_INITIALIZED", "SSAI not initialized", true)
        return parser.parseHLS(java.net.URL(url))
    }
}

/**
 * Builder for SSAIConfig
 */
class SSAIConfigBuilder {
    private var endpoint: String? = null
    private var tenantId: String? = null
    private var contentUrl: String? = null
    private var format: SSAIContentFormat = SSAIContentFormat.HLS
    private var capabilities: SSAIDeviceCapabilities? = null
    private var timeoutMs: Long = 10000L
    private var debug: Boolean = false

    fun setEndpoint(endpoint: String) = apply { this.endpoint = endpoint }
    fun setTenantId(tenantId: String) = apply { this.tenantId = tenantId }
    fun setContentUrl(contentUrl: String) = apply { this.contentUrl = contentUrl }
    fun setFormat(format: SSAIContentFormat) = apply { this.format = format }
    fun setCapabilities(capabilities: SSAIDeviceCapabilities) = apply { this.capabilities = capabilities }
    fun useDefaultAndroidTvCapabilities() = apply { this.capabilities = SSAIDeviceCapabilities.androidTv }
    fun setTimeout(timeoutMs: Long) = apply { this.timeoutMs = timeoutMs }
    fun setDebug(debug: Boolean) = apply { this.debug = debug }

    fun build(): SSAIConfig {
        val endpoint = this.endpoint
            ?: throw SSAIError("MISSING_ENDPOINT", "Endpoint is required", true)
        val tenantId = this.tenantId
            ?: throw SSAIError("MISSING_TENANT_ID", "Tenant ID is required", true)
        val contentUrl = this.contentUrl
            ?: throw SSAIError("MISSING_CONTENT_URL", "Content URL is required", true)

        return SSAIConfig(
            endpoint = endpoint,
            tenantId = tenantId,
            contentUrl = contentUrl,
            format = format,
            capabilities = capabilities ?: SSAIDeviceCapabilities.androidTv,
            timeoutMs = timeoutMs,
            debug = debug
        )
    }
}

/**
 * Extension function to create config builder
 */
fun ssaiConfig(block: SSAIConfigBuilder.() -> Unit): SSAIConfig {
    return SSAIConfigBuilder().apply(block).build()
}
