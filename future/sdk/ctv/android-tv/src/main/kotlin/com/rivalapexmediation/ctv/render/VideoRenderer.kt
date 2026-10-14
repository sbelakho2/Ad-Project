package com.rivalapexmediation.ctv.render

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import com.rivalapexmediation.ctv.metrics.MetricsRecorder
import com.rivalapexmediation.ctv.util.Logger
import java.util.EnumSet
import kotlin.math.min

internal enum class VideoProgressEvent {
    START,
    FIRST_QUARTILE,
    MIDPOINT,
    THIRD_QUARTILE,
    COMPLETE,
    PAUSE,
    RESUME,
    MUTE,
    UNMUTE,
    CLOSE,
}

internal class VideoRenderer(private val context: Context) {
    private var player: ExoPlayer? = null
    private var listener: Player.Listener? = null
    private val handler = Handler(Looper.getMainLooper())
    private var trackingListener: ((VideoProgressEvent) -> Unit)? = null
    private val emittedEvents: EnumSet<VideoProgressEvent> = EnumSet.noneOf(VideoProgressEvent::class.java)
    private var lastMuteState: Boolean = false
    private val progressRunnable = object: Runnable {
        override fun run() {
            val p = player ?: return
            if (p.playbackState == Player.STATE_IDLE || p.playbackState == Player.STATE_ENDED) return
            val duration = p.duration
            if (duration > 0) {
                val position = p.currentPosition
                val startThreshold = min(2000L, (duration * 0.05).toLong())
                if (position >= startThreshold) emit(VideoProgressEvent.START)
                val percent = position.toDouble() / duration.toDouble()
                if (percent >= 0.25) emit(VideoProgressEvent.FIRST_QUARTILE)
                if (percent >= 0.50) emit(VideoProgressEvent.MIDPOINT)
                if (percent >= 0.75) emit(VideoProgressEvent.THIRD_QUARTILE)
            }
            handler.postDelayed(this, 250)
        }
    }

    fun attach(playerView: PlayerView) {
        if (player == null) player = ExoPlayer.Builder(context).build()
        playerView.player = player
    }

    fun play(
        url: String,
        trackingCallback: ((VideoProgressEvent) -> Unit)? = null,
        onReady: (() -> Unit)? = null,
        onEnded: (() -> Unit)? = null,
    ) {
        val p = player ?: return
        trackingListener = trackingCallback
        emittedEvents.clear()
        handler.removeCallbacks(progressRunnable)
        listener?.let { p.removeListener(it) }
        try {
            p.setMediaItem(MediaItem.fromUri(url))
            p.prepare()
            p.playWhenReady = true
            val l = object: Player.Listener {
                override fun onRenderedFirstFrame() {
                    onReady?.invoke()
                }
                override fun onPlaybackStateChanged(state: Int) {
                    if (state == Player.STATE_READY) {
                        handler.post(progressRunnable)
                    }
                    if (state == Player.STATE_ENDED) {
                        emit(VideoProgressEvent.COMPLETE)
                        handler.removeCallbacks(progressRunnable)
                        onEnded?.invoke()
                    }
                }
                override fun onIsPlayingChanged(isPlaying: Boolean) {
                    if (isPlaying) {
                        emit(VideoProgressEvent.RESUME)
                        handler.post(progressRunnable)
                    } else {
                        emit(VideoProgressEvent.PAUSE)
                        handler.removeCallbacks(progressRunnable)
                    }
                }
                override fun onEvents(player: Player, events: Player.Events) {
                    if (events.contains(Player.EVENT_DEVICE_VOLUME_CHANGED)) {
                        val muted = player.isDeviceMuted || player.volume == 0f
                        if (muted != lastMuteState) {
                            lastMuteState = muted
                            emit(if (muted) VideoProgressEvent.MUTE else VideoProgressEvent.UNMUTE)
                        }
                    }
                }
            }
            lastMuteState = p.isDeviceMuted || p.volume == 0f
            listener = l
            p.addListener(l)
        } catch (e: Exception) {
            Logger.w("VideoRenderer error: ${e.message}", e)
        }
    }

    fun stop() {
        emit(VideoProgressEvent.CLOSE)
        handler.removeCallbacks(progressRunnable)
        try { player?.stop() } catch (_: Throwable) {}
    }

    fun release() {
        emit(VideoProgressEvent.CLOSE)
        listener?.let { player?.removeListener(it) }
        listener = null
        handler.removeCallbacks(progressRunnable)
        trackingListener = null
        emittedEvents.clear()
        try { player?.release() } catch (_: Throwable) {}
        player = null
    }

    private fun emit(event: VideoProgressEvent) {
        if (emittedEvents.contains(event)) return
        emittedEvents.add(event)
        trackingListener?.invoke(event)
        MetricsRecorder.recordPlayback(event)
    }
}
