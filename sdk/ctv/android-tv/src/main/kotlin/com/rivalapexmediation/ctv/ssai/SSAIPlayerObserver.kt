package com.rivalapexmediation.ctv.ssai

import android.os.Handler
import android.os.Looper
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

/**
 * SSAI Player Observer for Android TV SDK
 *
 * Monitors ExoPlayer for SSAI ad playback and automatically tracks events.
 */
class SSAIPlayerObserver(
    private val sessionClient: SSAISessionClient,
    private val config: ObserverConfig = ObserverConfig()
) : Player.Listener {

    /**
     * Observer configuration
     */
    data class ObserverConfig(
        /** Interval for time observation (milliseconds) */
        val timeObserverIntervalMs: Long = 250L,
        /** Enable automatic quartile tracking */
        val autoTrackQuartiles: Boolean = true,
        /** Enable automatic impression tracking */
        val autoTrackImpressions: Boolean = true,
        /** Tolerance for ad boundary detection (milliseconds) */
        val boundaryToleranceMs: Long = 500L
    )

    /**
     * Observer listener
     */
    interface Listener {
        fun onEnterAdBreak(adBreak: SSAIAdBreak) {}
        fun onExitAdBreak(adBreak: SSAIAdBreak) {}
        fun onAdProgress(ad: SSAIAd, progress: Double) {}
        fun onAdComplete(ad: SSAIAd) {}
        fun onAdSkip(ad: SSAIAd) {}
        fun onError(error: SSAIError, ad: SSAIAd?) {}
    }

    // Properties
    private var player: ExoPlayer? = null
    private val manifestParser = SSAIManifestParser()
    private val mainHandler = Handler(Looper.getMainLooper())
    private var scope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private var timeUpdateJob: Job? = null

    private var adCues = listOf<SSAIManifestParser.AdCue>()
    private var currentAdBreak: SSAIAdBreak? = null
    private var currentAd: SSAIAd? = null
    private var currentAdProgress = 0.0
    private val trackedQuartiles = mutableSetOf<String>()
    private val impressionsFired = mutableSetOf<String>()
    
    private var isPlaying = false
    private var lastPlaybackTimeMs = 0L

    var listener: Listener? = null

    // Flows for reactive subscriptions
    private val _adBreakEvents = MutableSharedFlow<SSAIAdBreak>()
    private val _adProgressEvents = MutableSharedFlow<Pair<SSAIAd, Double>>()
    
    val adBreakEvents: SharedFlow<SSAIAdBreak> = _adBreakEvents.asSharedFlow()
    val adProgressEvents: SharedFlow<Pair<SSAIAd, Double>> = _adProgressEvents.asSharedFlow()

    // Public accessors
    val isInAdBreak: Boolean get() = currentAdBreak != null
    val currentAdInfo: SSAIAd? get() = currentAd
    val currentAdBreakInfo: SSAIAdBreak? get() = currentAdBreak

    /**
     * Attach to ExoPlayer for observation
     */
    fun attach(player: ExoPlayer) {
        detach()
        this.player = player
        player.addListener(this)
        startTimeUpdates()
    }

    /**
     * Detach from current player
     */
    fun detach() {
        stopTimeUpdates()
        player?.removeListener(this)
        player = null
        currentAdBreak = null
        currentAd = null
        currentAdProgress = 0.0
        trackedQuartiles.clear()
        impressionsFired.clear()
    }

    /**
     * Load ad cues from manifest URL
     */
    suspend fun loadAdCues(manifestUrl: String) {
        val result = manifestParser.parseHLS(java.net.URL(manifestUrl))
        adCues = result.adCues
    }

    /**
     * Set ad cues directly
     */
    fun setAdCues(cues: List<SSAIManifestParser.AdCue>) {
        adCues = cues
    }

    /**
     * Set ad breaks from session
     */
    fun setAdBreaks(adBreaks: List<SSAIAdBreak>) {
        var slotIndex = 0
        adCues = adBreaks.flatMap { adBreak ->
            adBreak.ads.map { ad ->
                val cue = SSAIManifestParser.AdCue(
                    id = ad.creativeId,
                    startTime = adBreak.startTime + ad.startOffset,
                    duration = ad.duration,
                    podId = adBreak.podId,
                    slotIndex = slotIndex++,
                    cueType = when (adBreak.type) {
                        "preroll" -> SSAIManifestParser.CueType.PREROLL
                        "postroll" -> SSAIManifestParser.CueType.POSTROLL
                        else -> SSAIManifestParser.CueType.MIDROLL
                    },
                    metadata = emptyMap()
                )
                cue
            }
        }
    }

    /**
     * Manually trigger skip (for skip button handling)
     */
    fun skipCurrentAd() {
        val ad = currentAd ?: return
        val adBreak = currentAdBreak ?: return

        sessionClient.trackSkip(
            podId = adBreak.podId,
            slotId = ad.slotId,
            creativeId = ad.creativeId,
            mediaTime = lastPlaybackTimeMs / 1000.0
        )

        listener?.onAdSkip(ad)

        currentAd = null
        currentAdProgress = 0.0
    }

    // Player.Listener implementation

    override fun onIsPlayingChanged(isCurrentlyPlaying: Boolean) {
        isPlaying = isCurrentlyPlaying
    }

    override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
        val ssaiError = SSAIError(
            code = "PLAYER_ERROR",
            message = error.message ?: "Unknown player error",
            fatal = true
        )
        listener?.onError(ssaiError, currentAd)
    }

    override fun onPlaybackStateChanged(playbackState: Int) {
        when (playbackState) {
            Player.STATE_ENDED -> {
                // Handle playback end
                currentAdBreak?.let { exitCurrentAdBreak() }
            }
        }
    }

    // Private methods

    private fun startTimeUpdates() {
        timeUpdateJob = scope.launch {
            while (isActive) {
                delay(config.timeObserverIntervalMs)
                player?.let { p ->
                    if (p.isPlaying) {
                        handleTimeUpdate(p.currentPosition)
                    }
                }
            }
        }
    }

    private fun stopTimeUpdates() {
        timeUpdateJob?.cancel()
        timeUpdateJob = null
    }

    private fun handleTimeUpdate(positionMs: Long) {
        lastPlaybackTimeMs = positionMs
        val positionSec = positionMs / 1000.0

        // Check if we're in an ad
        val currentCue = findCurrentCue(positionSec)

        if (currentCue != null) {
            handleAdPlayback(currentCue, positionSec)
        } else if (currentAdBreak != null) {
            exitCurrentAdBreak()
        }
    }

    private fun findCurrentCue(timeSec: Double): SSAIManifestParser.AdCue? {
        val toleranceSec = config.boundaryToleranceMs / 1000.0
        
        return adCues.find { cue ->
            val adStart = cue.startTime - toleranceSec
            val adEnd = cue.startTime + cue.duration + toleranceSec
            timeSec in adStart..adEnd
        }
    }

    private fun handleAdPlayback(cue: SSAIManifestParser.AdCue, currentTimeSec: Double) {
        // Create ad break if needed
        if (currentAdBreak == null) {
            enterAdBreak(cue)
        }

        // Create current ad if needed
        if (currentAd == null) {
            enterAd(cue)
        }

        // Calculate progress
        val ad = currentAd ?: return
        val adStartTime = cue.startTime
        val adDuration = cue.duration

        if (adDuration <= 0) return

        val adElapsed = currentTimeSec - adStartTime
        val progress = minOf(100.0, maxOf(0.0, (adElapsed / adDuration) * 100))

        currentAdProgress = progress
        
        scope.launch {
            _adProgressEvents.emit(ad to progress)
        }
        listener?.onAdProgress(ad, progress)

        // Auto-track quartiles
        if (config.autoTrackQuartiles) {
            trackQuartilesIfNeeded(ad, progress, cue)
        }

        // Check for ad completion
        if (progress >= 100) {
            completeCurrentAd(cue)
        }
    }

    private fun enterAdBreak(cue: SSAIManifestParser.AdCue) {
        // Find all ads in this pod
        val podAds = adCues.filter { it.podId == cue.podId }

        val ads = podAds.map { podCue ->
            SSAIAd(
                slotId = "slot-${podCue.slotIndex}",
                creativeId = podCue.id,
                duration = podCue.duration,
                startOffset = podCue.startTime - cue.startTime,
                skipOffset = null,
                vastUrl = null,
                verificationScripts = emptyList()
            )
        }

        val totalDuration = podAds.sumOf { it.duration }

        val adBreak = SSAIAdBreak(
            podId = cue.podId,
            type = cue.cueType.value,
            position = cue.startTime.toInt(),
            startTime = cue.startTime,
            duration = totalDuration,
            plannedSlots = ads.size,
            filledSlots = ads.size,
            ads = ads
        )

        currentAdBreak = adBreak
        
        scope.launch {
            _adBreakEvents.emit(adBreak)
        }
        listener?.onEnterAdBreak(adBreak)
    }

    private fun enterAd(cue: SSAIManifestParser.AdCue) {
        val ad = SSAIAd(
            slotId = "slot-${cue.slotIndex}",
            creativeId = cue.id,
            duration = cue.duration,
            startOffset = 0.0,
            skipOffset = null,
            vastUrl = null,
            verificationScripts = emptyList()
        )

        currentAd = ad
        currentAdProgress = 0.0

        // Track impression
        if (config.autoTrackImpressions) {
            val impressionKey = "${cue.podId}:${ad.creativeId}"
            if (impressionKey !in impressionsFired) {
                impressionsFired.add(impressionKey)

                sessionClient.trackImpression(
                    podId = cue.podId,
                    slotId = ad.slotId,
                    creativeId = ad.creativeId,
                    mediaTime = cue.startTime
                )
            }
        }
    }

    private fun trackQuartilesIfNeeded(ad: SSAIAd, progress: Double, cue: SSAIManifestParser.AdCue) {
        val adBreak = currentAdBreak ?: return

        val quartiles = listOf(
            0.0 to SSAITrackingEventType.START,
            25.0 to SSAITrackingEventType.FIRST_QUARTILE,
            50.0 to SSAITrackingEventType.MIDPOINT,
            75.0 to SSAITrackingEventType.THIRD_QUARTILE
        )

        for ((threshold, eventType) in quartiles) {
            val quartileKey = "${adBreak.podId}:${ad.creativeId}:${eventType.value}"

            if (progress >= threshold && quartileKey !in trackedQuartiles) {
                trackedQuartiles.add(quartileKey)

                sessionClient.trackProgress(
                    podId = adBreak.podId,
                    slotId = ad.slotId,
                    creativeId = ad.creativeId,
                    mediaTime = lastPlaybackTimeMs / 1000.0,
                    progress = threshold
                )
            }
        }
    }

    private fun completeCurrentAd(cue: SSAIManifestParser.AdCue) {
        val ad = currentAd ?: return
        val adBreak = currentAdBreak ?: return

        // Track complete
        val completeKey = "${adBreak.podId}:${ad.creativeId}:complete"
        if (completeKey !in trackedQuartiles) {
            trackedQuartiles.add(completeKey)

            sessionClient.trackProgress(
                podId = adBreak.podId,
                slotId = ad.slotId,
                creativeId = ad.creativeId,
                mediaTime = lastPlaybackTimeMs / 1000.0,
                progress = 100.0
            )
        }

        listener?.onAdComplete(ad)

        // Check if there are more ads in the pod
        val currentIndex = cue.slotIndex
        val nextCue = adCues.find { it.podId == cue.podId && it.slotIndex == currentIndex + 1 }

        if (nextCue == null) {
            // No more ads, exit ad break
            exitCurrentAdBreak()
        } else {
            // More ads, clear current ad for next one
            currentAd = null
            currentAdProgress = 0.0
        }
    }

    private fun exitCurrentAdBreak() {
        val adBreak = currentAdBreak ?: return

        listener?.onExitAdBreak(adBreak)

        currentAdBreak = null
        currentAd = null
        currentAdProgress = 0.0
    }
}
