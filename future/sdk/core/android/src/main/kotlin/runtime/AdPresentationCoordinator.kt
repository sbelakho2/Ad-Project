package com.rivalapexmediation.sdk.runtime

import android.app.Activity
import android.app.Application
import android.os.Handler
import android.os.Looper
import androidx.annotation.VisibleForTesting
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import com.rivalapexmediation.sdk.logging.Logger
import com.rivalapexmediation.sdk.models.AdType
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.max

/**
 * Coordinates ad presentations so we only ever show one fullscreen ad at a time, and
 * only when the host Activity is in a resumed/foreground state.
 */
internal object AdPresentationCoordinator {
    private const val TAG = "AdPresentationCoord"
    private const val RESUME_TIMEOUT_MS = 2_000L
    private const val FOREGROUND_TIMEOUT_MS = 4_000L

    private val requestIds = AtomicLong(0)
    private val currentRequest = AtomicReference<PendingRequest?>()
    private val mainHandler = Handler(Looper.getMainLooper())
    private val tracker = ActivityTracker(mainHandler)

    fun begin(
        activity: Activity,
        placementId: String,
        adType: AdType,
        block: (Activity) -> Boolean,
    ): Boolean {
        ensureRegistered(activity.application)
        tracker.prime(activity)
        val request = PendingRequest(
            id = requestIds.incrementAndGet(),
            placementId = placementId,
            adType = adType,
            componentKey = activityComponentKey(activity),
            action = block,
        )
        if (!currentRequest.compareAndSet(null, request)) {
            Logger.w(TAG, "presentation already in flight; placement=$placementId")
            return false
        }

        val readyActivity = if (tracker.isActivityReady(activity)) {
            activity
        } else {
            tracker.findLastKnownActivity(request.componentKey)
        }

        if (readyActivity != null && tracker.isActivityReady(readyActivity)) {
            dispatch(request, readyActivity)
        } else {
            tracker.awaitResume(
                request = request,
                timeoutMs = RESUME_TIMEOUT_MS,
                onReady = { resumed -> dispatch(request, resumed) },
                onDestroyed = { fail(request, "activity_destroyed") },
                onTimeout = { fail(request, "activity_resume_timeout") }
            )
        }
        return true
    }

    private fun dispatch(request: PendingRequest, targetActivity: Activity) {
        tracker.runWhenForeground(
            request = request,
            timeoutMs = FOREGROUND_TIMEOUT_MS,
            onReady = {
                val runnable = Runnable {
                    val success = try {
                        request.action(targetActivity)
                    } catch (t: Throwable) {
                        Logger.e(TAG, "uncaught error while presenting ${request.placementId}", t)
                        false
                    }
                    finish(request)
                    if (!success) {
                        Logger.w(TAG, "presentation block returned false for ${request.placementId}")
                    }
                }
                if (Looper.myLooper() == Looper.getMainLooper()) {
                    runnable.run()
                } else {
                    mainHandler.post(runnable)
                }
            },
            onTimeout = { fail(request, "foreground_timeout") }
        )
    }

    private fun finish(request: PendingRequest) {
        tracker.clearPending(request.id)
        currentRequest.compareAndSet(request, null)
    }

    private fun fail(request: PendingRequest, reason: String) {
        tracker.clearPending(request.id)
        if (currentRequest.compareAndSet(request, null)) {
            Logger.w(TAG, "dropping presentation for ${request.placementId} (${request.adType}): $reason")
        }
    }

    private fun ensureRegistered(application: Application) {
        tracker.ensure(application)
    }

    @VisibleForTesting
    fun resetForTesting() {
        currentRequest.set(null)
        tracker.reset()
        mainHandler.removeCallbacksAndMessages(null)
    }

    private data class PendingRequest(
        val id: Long,
        val placementId: String,
        val adType: AdType,
        val componentKey: String,
        val action: (Activity) -> Boolean,
    )

    private class ActivityTracker(private val handler: Handler) : Application.ActivityLifecycleCallbacks {
        private val registered = AtomicBoolean(false)
        private var application: Application? = null
        private val lock = Any()
        private val resumedComponents = mutableSetOf<String>()
        private val lastKnownActivities = mutableMapOf<String, WeakReference<Activity>>()
        private var pendingResume: ResumeWaiter? = null
        private var pendingForeground: ForegroundWaiter? = null
        private var startedCount = 0

        fun ensure(app: Application) {
            if (registered.compareAndSet(false, true)) {
                application = app
                app.registerActivityLifecycleCallbacks(this)
            }
        }

        fun reset() {
            val app = application
            if (app != null && registered.get()) {
                app.unregisterActivityLifecycleCallbacks(this)
            }
            registered.set(false)
            synchronized(lock) {
                resumedComponents.clear()
                lastKnownActivities.clear()
                pendingResume = null
                pendingForeground = null
                startedCount = 0
            }
        }

        fun isActivityReady(activity: Activity): Boolean {
            val key = activityComponentKey(activity)
            val trackedReady = synchronized(lock) { resumedComponents.contains(key) }
            return (trackedReady || isVisiblyReady(activity)) && !activity.isFinishing && !activity.isDestroyed
        }

        fun prime(activity: Activity) {
            if (!isVisiblyReady(activity)) return
            val key = activityComponentKey(activity)
            synchronized(lock) {
                resumedComponents.add(key)
                lastKnownActivities[key] = WeakReference(activity)
                if (startedCount == 0) {
                    startedCount = 1
                }
            }
        }

        fun findLastKnownActivity(componentKey: String): Activity? {
            return synchronized(lock) { lastKnownActivities[componentKey]?.get() }
        }

        fun awaitResume(
            request: PendingRequest,
            timeoutMs: Long,
            onReady: (Activity) -> Unit,
            onDestroyed: () -> Unit,
            onTimeout: () -> Unit,
        ) {
            val cached = findLastKnownActivity(request.componentKey)
            if (cached != null && isActivityReady(cached)) {
                onReady(cached)
                return
            }

            val timeoutRunnable = Runnable {
                val shouldTimeout = synchronized(lock) {
                    if (pendingResume?.requestId == request.id) {
                        pendingResume = null
                        true
                    } else {
                        false
                    }
                }
                if (shouldTimeout) {
                    onTimeout()
                }
            }
            val waiter = ResumeWaiter(
                requestId = request.id,
                componentKey = request.componentKey,
                onReady = onReady,
                onDestroyed = onDestroyed,
                timeout = timeoutRunnable,
            )
            synchronized(lock) { pendingResume = waiter }
            handler.postDelayed(timeoutRunnable, timeoutMs)
        }

        fun runWhenForeground(
            request: PendingRequest,
            timeoutMs: Long,
            onReady: () -> Unit,
            onTimeout: () -> Unit,
        ) {
            val immediate = synchronized(lock) {
                if (startedCount > 0 || resumedComponents.isNotEmpty()) {
                    true
                } else {
                    val timeoutRunnable = Runnable {
                        val shouldTimeout = synchronized(lock) {
                            if (pendingForeground?.requestId == request.id) {
                                pendingForeground = null
                                true
                            } else {
                                false
                            }
                        }
                        if (shouldTimeout) {
                            onTimeout()
                        }
                    }
                    val waiter = ForegroundWaiter(request.id, onReady, timeoutRunnable)
                    pendingForeground = waiter
                    handler.postDelayed(timeoutRunnable, timeoutMs)
                    false
                }
            }
            if (immediate) {
                onReady()
            }
        }

        fun clearPending(requestId: Long) {
            synchronized(lock) {
                if (pendingResume?.requestId == requestId) {
                    pendingResume?.let { handler.removeCallbacks(it.timeout) }
                    pendingResume = null
                }
                if (pendingForeground?.requestId == requestId) {
                    pendingForeground?.let { handler.removeCallbacks(it.timeout) }
                    pendingForeground = null
                }
            }
        }

        override fun onActivityCreated(activity: Activity, savedInstanceState: android.os.Bundle?) {}

        override fun onActivityStarted(activity: Activity) {
            val callback = synchronized(lock) {
                startedCount++
                if (startedCount == 1) {
                    val waiter = pendingForeground
                    pendingForeground = null
                    waiter?.let { handler.removeCallbacks(it.timeout) }
                    waiter
                } else {
                    null
                }
            }
            callback?.action?.invoke()
        }

        override fun onActivityResumed(activity: Activity) {
            val key = activityComponentKey(activity)
            val callback = synchronized(lock) {
                resumedComponents.add(key)
                lastKnownActivities[key] = WeakReference(activity)
                if (pendingResume?.componentKey == key) {
                    val waiter = pendingResume
                    pendingResume = null
                    waiter?.let { handler.removeCallbacks(it.timeout) }
                    waiter
                } else {
                    null
                }
            }
            callback?.onReady?.invoke(activity)
        }

        override fun onActivityPaused(activity: Activity) {
            val key = activityComponentKey(activity)
            synchronized(lock) { resumedComponents.remove(key) }
        }

        override fun onActivityStopped(activity: Activity) {
            synchronized(lock) { startedCount = max(0, startedCount - 1) }
        }

        override fun onActivitySaveInstanceState(activity: Activity, outState: android.os.Bundle) {}

        override fun onActivityDestroyed(activity: Activity) {
            val key = activityComponentKey(activity)
            val destroyed = synchronized(lock) {
                lastKnownActivities.remove(key)
                if (!activity.isChangingConfigurations && pendingResume?.componentKey == key) {
                    val waiter = pendingResume
                    pendingResume = null
                    waiter?.let { handler.removeCallbacks(it.timeout) }
                    waiter
                } else {
                    null
                }
            }
            destroyed?.onDestroyed?.invoke()
        }

        private data class ResumeWaiter(
            val requestId: Long,
            val componentKey: String,
            val onReady: (Activity) -> Unit,
            val onDestroyed: () -> Unit,
            val timeout: Runnable,
        )

        private data class ForegroundWaiter(
            val requestId: Long,
            val action: () -> Unit,
            val timeout: Runnable,
        )

        private fun isVisiblyReady(activity: Activity): Boolean {
            val lifecycleReady = (activity as? LifecycleOwner)?.lifecycle?.currentState?.isAtLeast(Lifecycle.State.RESUMED) ?: false
            val window = activity.window
            val decor = window?.decorView
            val windowReady = (decor?.isShown == true) || (decor?.hasWindowFocus() == true) || activity.hasWindowFocus()
            return lifecycleReady || windowReady
        }
    }
}

private fun activityComponentKey(activity: Activity): String {
    return activity.componentName?.flattenToShortString() ?: activity.javaClass.name
}
