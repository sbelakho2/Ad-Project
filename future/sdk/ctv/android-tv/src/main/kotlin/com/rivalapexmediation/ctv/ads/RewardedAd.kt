package com.rivalapexmediation.ctv.ads

import android.app.Activity
import androidx.media3.ui.PlayerView
import com.rivalapexmediation.ctv.ApexMediation
import com.rivalapexmediation.ctv.network.AuctionWin
import com.rivalapexmediation.ctv.network.reason
import com.rivalapexmediation.ctv.render.Beacon
import com.rivalapexmediation.ctv.render.VideoProgressEvent
import com.rivalapexmediation.ctv.render.VideoRenderer
import com.rivalapexmediation.ctv.util.Logger

class RewardedAd(private val placementId: String) {
    @Volatile private var win: AuctionWin? = null
    @Volatile private var loaded = false

    fun load(floorCpm: Double = 0.0, callback: AdLoadCallback) {
        if (!ApexMediation.isInitialized()) { callback.onError("not_initialized"); return }
        ApexMediation.loadBlockedReason(placementId)?.let {
            callback.onError(it)
            return
        }
        val consent = ApexMediation.consent()
        ApexMediation.client().requestBid(placementId, "rewarded", floorCpm, consent) { result ->
            result.error?.let {
                ApexMediation.recordLoadFailure(placementId, it.reason())
                ApexMediation.postMain { callback.onError(it.reason()) }
                return@requestBid
            }
            val nextWin = result.win
            loaded = nextWin != null
            if (nextWin != null) {
                AdCache.put(placementId, nextWin)
                win = nextWin
                ApexMediation.recordLoadSuccess(placementId)
                ApexMediation.postMain { callback.onLoaded() }
            } else {
                ApexMediation.recordLoadFailure(placementId, "no_fill")
                ApexMediation.postMain { callback.onError("no_fill") }
            }
        }
    }

    fun isReady(): Boolean = AdCache.peek(placementId) != null

    fun show(activity: Activity, playerView: PlayerView, callback: AdShowCallback) {
        ApexMediation.showBlockedReason(placementId)?.let {
            callback.onError(it)
            return
        }
        val cached = AdCache.take(placementId)
        val w = when {
            cached != null -> cached
            loaded -> {
                loaded = false
                callback.onError("expired")
                return
            }
            else -> {
                callback.onError("not_ready")
                return
            }
        }
        win = w
        loaded = false
        val renderer = VideoRenderer(activity)
        renderer.attach(playerView)
        try {
            renderer.play(
                w.creativeUrl,
                trackingCallback = { event ->
                    when (event) {
                        VideoProgressEvent.START -> Beacon.fire(w.tracking.start, "start")
                        VideoProgressEvent.FIRST_QUARTILE -> Beacon.fire(w.tracking.firstQuartile, "first_quartile")
                        VideoProgressEvent.MIDPOINT -> Beacon.fire(w.tracking.midpoint, "midpoint")
                        VideoProgressEvent.THIRD_QUARTILE -> Beacon.fire(w.tracking.thirdQuartile, "third_quartile")
                        VideoProgressEvent.COMPLETE -> Beacon.fire(w.tracking.complete, "complete")
                        VideoProgressEvent.PAUSE -> Beacon.fire(w.tracking.pause, "pause")
                        VideoProgressEvent.RESUME -> Beacon.fire(w.tracking.resume, "resume")
                        VideoProgressEvent.MUTE -> Beacon.fire(w.tracking.mute, "mute")
                        VideoProgressEvent.UNMUTE -> Beacon.fire(w.tracking.unmute, "unmute")
                        VideoProgressEvent.CLOSE -> Beacon.fire(w.tracking.close, "close")
                    }
                },
                onReady = {
                    try { Beacon.fire(w.tracking.impression, "impression") } catch (_: Throwable) {}
                    ApexMediation.postMain { callback.onShown() }
                },
                onEnded = {
                    ApexMediation.postMain { callback.onClosed() }
                    renderer.release()
                }
            )
        } catch (e: Exception) {
            Logger.w("Rewarded show error: ${e.message}", e)
            callback.onError("render_error")
        }
    }

    /**
     * Report a user click for this ad (fires signed click beacon).
     */
    fun reportClick() {
        val w = win ?: return
        try { Beacon.fire(w.tracking.click, "click") } catch (_: Throwable) {}
    }
}
