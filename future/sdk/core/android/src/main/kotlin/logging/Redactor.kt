package com.rivalapexmediation.sdk.logging

/**
 * Pure-JVM redaction utilities (no Android dependencies) so we can unit-test easily.
 */
object Redactor {
    private val apiKeyHeaderRegex = Regex("(?i)x-api-key[:=]\\s*([A-Za-z0-9_\\n\\r\\-]{4,})")
    private val kvSecretRegex = Regex("(?i)(api[_-]?key|secret|token)=([A-Za-z0-9\\-_.]{6,})")

    fun redactSecrets(input: String?): String {
        if (input == null) return ""
        return input
            .replace(apiKeyHeaderRegex) { m ->
                val key = m.groupValues.getOrNull(1) ?: return@replace m.value
                val masked = mask(key)
                m.value.replace(key, masked)
            }
            .replace(kvSecretRegex) { m ->
                val valPart = m.groupValues.getOrNull(2) ?: return@replace m.value
                val masked = mask(valPart)
                m.value.replace(valPart, masked)
            }
    }

    private fun mask(s: String): String {
        return if (s.length <= 6) "***" else s.take(3) + "***" + s.takeLast(3)
    }
}
