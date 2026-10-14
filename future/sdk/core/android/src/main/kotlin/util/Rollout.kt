package com.rivalapexmediation.sdk.util

import android.content.Context
import java.security.MessageDigest

/**
 * Deterministic per-install rollout bucketing utilities.
 * - Uses stable InstallId (non-PII) and SHA-256 hashing
 * - Maps install to bucket [0,99]
 */
object Rollout {
    @JvmStatic
    fun bucket(context: Context): Int {
        val id = InstallId.get(context)
        val digest = MessageDigest.getInstance("SHA-256").digest(id.toByteArray())
        // Use first 4 bytes as unsigned int to reduce bias, then mod 100
        val v = ((digest[0].toInt() and 0xFF) shl 24) or
                ((digest[1].toInt() and 0xFF) shl 16) or
                ((digest[2].toInt() and 0xFF) shl 8) or
                (digest[3].toInt() and 0xFF)
        val asLong = v.toLong() and 0xFFFFFFFFL
        return (asLong % 100).toInt()
    }

    @JvmStatic
    fun isInRollout(context: Context, percentage: Int): Boolean {
        val pct = when {
            percentage <= 0 -> 0
            percentage >= 100 -> 100
            else -> percentage
        }
        return bucket(context) < pct
    }
}
