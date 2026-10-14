package com.rivalapexmediation.sdk.config

import com.rivalapexmediation.sdk.models.FeatureFlags
import com.rivalapexmediation.sdk.models.SDKRemoteConfig
import org.junit.Assert.assertEquals
import org.junit.Test

class ConfigCanonicalMessageTest {

    private fun config(configId: String, version: Long, timestamp: Long) = SDKRemoteConfig(
        configId = configId,
        version = version,
        placements = emptyMap(),
        adapters = emptyMap(),
        features = FeatureFlags(),
        signature = "sig",
        timestamp = timestamp,
    )

    @Test
    fun canonical_message_pins_the_cross_platform_byte_order() {
        // Byte-identical to the iOS SignatureVerifier.createCanonicalMessage
        // test vector: {"config_id":"config_1","timestamp":1234,"version":4}
        assertEquals(
            "{\"config_id\":\"config_1\",\"timestamp\":1234,\"version\":4}",
            String(canonicalSigningMessage(config("config_1", 4, 1234)), Charsets.UTF_8),
        )
    }

    @Test
    fun canonical_message_escapes_ids_that_contain_quotes() {
        assertEquals(
            "{\"config_id\":\"cfg\\\"quote\",\"timestamp\":9,\"version\":1}",
            String(canonicalSigningMessage(config("cfg\"quote", 1, 9)), Charsets.UTF_8),
        )
    }
}
