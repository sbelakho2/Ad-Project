package com.rivalapexmediation.ctv.ads

import android.os.SystemClock

internal interface Clock {
    fun now(): Long // monotonic milliseconds
}

internal object SystemMonotonicClock : Clock {
    override fun now(): Long = SystemClock.elapsedRealtime()
}
