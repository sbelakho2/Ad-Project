package com.rivalapexmediation.sdk.privacy

import android.content.Context
import com.google.android.gms.ads.identifier.AdvertisingIdClient
import com.google.android.gms.appset.AppSet
import com.google.android.gms.tasks.Tasks
import java.util.concurrent.TimeUnit

/**
 * Snapshot of device-scoped identifiers that may be forwarded to adapters when privacy allows it.
 * Values are always redacted (null) when the user limits ad tracking or when fetches fail.
 */
data class PrivacyIdentifiers(
    val advertisingId: String? = null,
    val appSetId: String? = null,
    val limitAdTracking: Boolean = true,
    val source: Source = Source.NONE
) {
    enum class Source { NONE, GAID, APP_SET }

    val hasAdvertisingId: Boolean get() = !advertisingId.isNullOrBlank()
    val hasAppSetId: Boolean get() = !appSetId.isNullOrBlank()
}

/**
 * Best-effort provider for GAID/App Set identifiers with strict privacy defaults:
 * - Honors Google Play Services toggles (LAT).
 * - Falls back to App Set IDs on Android 12+ when GAID is unavailable.
 * - Never blocks the UI thread; callers must invoke from background executors.
 */
class PrivacyIdentifierProvider @JvmOverloads constructor(
    private val context: Context,
    private val gaidSupplier: (Context) -> PrivacyIdentifiers? = { defaultGaid(it) },
    private val appSetSupplier: (Context) -> PrivacyIdentifiers? = { defaultAppSet(it) }
) {

    fun collect(optOut: Boolean = false): PrivacyIdentifiers {
        if (optOut) {
            return PrivacyIdentifiers(limitAdTracking = true)
        }
        gaidSupplier(context)?.let { return it }
        appSetSupplier(context)?.let { return it }
        return PrivacyIdentifiers(limitAdTracking = true)
    }

    companion object {
        private val ZERO_UUID = "00000000-0000-0000-0000-000000000000"
        private const val APP_SET_TIMEOUT_MS = 1500L

        internal fun defaultGaid(context: Context): PrivacyIdentifiers? {
            return try {
                val info = AdvertisingIdClient.getAdvertisingIdInfo(context)
                when {
                    info == null -> null
                    info.isLimitAdTrackingEnabled -> PrivacyIdentifiers(limitAdTracking = true, source = PrivacyIdentifiers.Source.GAID)
                    else -> {
                        val id = info.id?.takeUnless { it.isNullOrZero() } ?: return null
                        PrivacyIdentifiers(
                            advertisingId = id,
                            limitAdTracking = false,
                            source = PrivacyIdentifiers.Source.GAID
                        )
                    }
                }
            } catch (_: Throwable) {
                null
            }
        }

        internal fun defaultAppSet(context: Context): PrivacyIdentifiers? {
            return try {
                val client = AppSet.getClient(context)
                val info = Tasks.await(client.appSetIdInfo, APP_SET_TIMEOUT_MS, TimeUnit.MILLISECONDS)
                val id = info?.id?.takeUnless { it.isNullOrZero() } ?: return null
                PrivacyIdentifiers(
                    appSetId = id,
                    limitAdTracking = false,
                    source = PrivacyIdentifiers.Source.APP_SET
                )
            } catch (_: Throwable) {
                null
            }
        }

        private fun String?.isNullOrZero(): Boolean {
            if (this.isNullOrBlank()) return true
            val normalized = this.lowercase()
            if (normalized == ZERO_UUID.lowercase()) return true
            return this.all { it == '0' || it == '-' }
        }
    }
}
