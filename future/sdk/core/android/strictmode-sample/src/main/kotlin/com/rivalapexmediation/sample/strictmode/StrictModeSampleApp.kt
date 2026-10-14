package com.rivalapexmediation.sample.strictmode

import android.app.Application
import android.os.StrictMode

/**
 * Minimal host application used for StrictMode smoke tests. Enables strict policies in debug builds.
 */
class StrictModeSampleApp : Application() {
    override fun onCreate() {
        super.onCreate()
        enableStrictMode()
    }

    private fun enableStrictMode() {
        StrictMode.setThreadPolicy(
            StrictMode.ThreadPolicy.Builder()
                .detectNetwork()
                .detectDiskReads()
                .detectDiskWrites()
                .detectCustomSlowCalls()
                .penaltyDeath()
                .penaltyLog()
                .build()
        )
        StrictMode.setVmPolicy(
            StrictMode.VmPolicy.Builder()
                .detectLeakedClosableObjects()
                .detectActivityLeaks()
                .detectLeakedSqlLiteObjects()
                .penaltyLog()
                .build()
        )
    }
}
