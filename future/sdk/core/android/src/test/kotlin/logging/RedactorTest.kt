package com.rivalapexmediation.sdk.logging

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RedactorTest {
    @Test
    fun masksApiKeyHeader() {
        val input = "X-Api-Key: SUPERSECRETKEYVALUE"
        val out = Redactor.redactSecrets(input)
        assertTrue(out.contains("X-Api-Key:"))
        assertFalse("should mask the key", out.contains("SUPERSECRETKEYVALUE"))
        assertTrue(out.contains("***"))
    }

    @Test
    fun masksKeyValueSecret() {
        val input = "token=myultrasecrettoken123"
        val out = Redactor.redactSecrets(input)
        assertFalse(out.contains("myultrasecrettoken123"))
        assertTrue(out.contains("token="))
        assertTrue(out.contains("***"))
    }
}
