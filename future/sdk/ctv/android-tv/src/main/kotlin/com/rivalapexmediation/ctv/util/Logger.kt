package com.rivalapexmediation.ctv.util

import android.util.Log

internal object Logger {
    private const val TAG = "CTVSDK"
    var debug: Boolean = true

    fun d(msg: String) { if (debug) Log.d(TAG, msg) }
    fun i(msg: String) { Log.i(TAG, msg) }
    fun w(msg: String, tr: Throwable? = null) { Log.w(TAG, msg, tr) }
    fun e(msg: String, tr: Throwable? = null) { Log.e(TAG, msg, tr) }
}
