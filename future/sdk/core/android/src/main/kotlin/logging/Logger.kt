package com.rivalapexmediation.sdk.logging

import android.util.Log
import com.rivalapexmediation.sdk.LogLevel

/**
 * Lightweight, centralized logger with runtime level control and redaction.
 * - No-op in release for VERBOSE/DEBUG to minimize overhead.
 * - Redacts known sensitive values.
 */
object Logger {
    @Volatile
    private var level: LogLevel = LogLevel.INFO

    fun setLevel(newLevel: LogLevel) {
        level = newLevel
    }

    private fun shouldLog(lvl: LogLevel): Boolean {
        return when (level) {
            LogLevel.VERBOSE -> true
            LogLevel.DEBUG -> lvl >= LogLevel.DEBUG
            LogLevel.INFO -> lvl >= LogLevel.INFO
            LogLevel.WARN -> lvl >= LogLevel.WARN
            LogLevel.ERROR -> lvl >= LogLevel.ERROR
        }
    }

    private fun redact(msg: String?): String {
        return com.rivalapexmediation.sdk.logging.Redactor.redactSecrets(msg)
    }

    fun v(tag: String, msg: String) { if (shouldLog(LogLevel.VERBOSE)) Log.v(tag, redact(msg)) }
    fun d(tag: String, msg: String) { if (shouldLog(LogLevel.DEBUG)) Log.d(tag, redact(msg)) }
    fun i(tag: String, msg: String) { if (shouldLog(LogLevel.INFO)) Log.i(tag, redact(msg)) }
    fun w(tag: String, msg: String, tr: Throwable? = null) { if (shouldLog(LogLevel.WARN)) Log.w(tag, redact(msg), tr) }
    fun e(tag: String, msg: String, tr: Throwable? = null) { if (shouldLog(LogLevel.ERROR)) Log.e(tag, redact(msg), tr) }
}
