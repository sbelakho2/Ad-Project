package com.rivalapexmediation.sdk.models

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class ValidationResultTest {
    @Test
    fun redactsSensitiveDetailKeys() {
        val result = ValidationResult.error(
            code = "missing_creds",
            message = "Provide runtime config",
            details = mapOf(
                "api_key" to "super-secret",
                "note" to "safe"
            )
        )

        assertEquals("***", result.details["api_key"])
        assertEquals("safe", result.details["note"])
    }

    @Test
    fun capsDetailEntriesToPreventLogSpam() {
        val noisy = (1..20).associate { "k$it" to "v$it" }
        val result = ValidationResult.ok(details = noisy)

        assertEquals(12, result.details.size)
        assertFalse(result.details.containsKey("k20"))
    }
}
