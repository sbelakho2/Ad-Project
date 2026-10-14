package com.rivalapexmediation.sdk.telemetry

import com.rivalapexmediation.sdk.logging.Redactor
import com.rivalapexmediation.sdk.models.TelemetryEvent

/**
 * Centralized telemetry sanitation to ensure BYO secrets never leave the device.
 */
internal object TelemetryRedactor {
    private val sensitiveKeys = setOf(
        "api_key",
        "app_key",
        "secret",
        "token",
        "account",
        "account_id",
        "credential",
        "credentials",
        "placement_id",
        "ad_unit",
        "app_id"
    )

    fun sanitize(event: TelemetryEvent): TelemetryEvent {
        val sanitizedMetadata = sanitizeMetadata(event.metadata)
        val sanitizedError = event.errorMessage?.let { Redactor.redactSecrets(it).take(256) }
        return event.copy(
            errorMessage = sanitizedError,
            metadata = sanitizedMetadata
        )
    }

    @Suppress("UNCHECKED_CAST")
    private fun sanitizeValue(value: Any?): Any? {
        return when (value) {
            is String -> Redactor.redactSecrets(value)
            is Map<*, *> -> sanitizeMetadata(value as Map<String, *>)
            is List<*> -> value.map { sanitizeValue(it) }
            else -> value
        }
    }

    private fun sanitizeMetadata(metadata: Map<String, *>): Map<String, Any> {
        if (metadata.isEmpty()) return emptyMap()
        val result = LinkedHashMap<String, Any>()
        metadata.forEach { (key, value) ->
            if (key.isBlank()) return@forEach
            val shouldMask = sensitiveKeys.any { key.contains(it, ignoreCase = true) }
            val sanitized = if (shouldMask) "***" else sanitizeValue(value)
            if (sanitized != null) {
                result[key] = sanitized
            }
        }
        return result
    }
}
