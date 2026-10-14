package com.rivalapexmediation.sdk.config

import android.content.Context
import android.content.SharedPreferences
import com.google.gson.Gson
import com.rivalapexmediation.sdk.SDKConfig
import com.rivalapexmediation.sdk.models.SDKRemoteConfig
import com.rivalapexmediation.sdk.models.FeatureFlags
import com.rivalapexmediation.sdk.models.PlacementConfig
import com.rivalapexmediation.sdk.models.AdType
import io.mockk.every
import io.mockk.mockk
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Before
import org.junit.Test
import java.security.KeyPairGenerator
import java.security.Signature
import java.util.Base64

class ConfigSignatureTest {
    private lateinit var server: MockWebServer

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    private fun gson() = Gson()

    private fun signingMessage(configId: String, version: Long, timestamp: Long): ByteArray {
        // Must mirror canonicalSigningMessage: lexicographic keys, matching the
        // iOS SignatureVerifier.createCanonicalMessage bytes.
        return ("{\"config_id\":\"$configId\",\"timestamp\":$timestamp,\"version\":$version}")
            .toByteArray()
    }

    private fun baseConfigJson(configId: String, version: Long, timestamp: Long, signatureB64: String): String {
        val placements = linkedMapOf(
            "pl1" to PlacementConfig(
                placementId = "pl1",
                adType = AdType.INTERSTITIAL,
                enabledNetworks = emptyList(),
                timeoutMs = 3000,
                maxWaitMs = 5000,
                floorPrice = 0.0,
                refreshInterval = null,
                targeting = com.rivalapexmediation.sdk.models.Targeting()
            )
        )
        val cfg = SDKRemoteConfig(
            configId = configId,
            version = version,
            placements = placements,
            adapters = emptyMap(),
            features = FeatureFlags(),
            signature = signatureB64,
            timestamp = timestamp
        )
        return gson().toJson(cfg)
    }

    private fun mockPrefs(): SharedPreferences {
        val editor = mockk<SharedPreferences.Editor>(relaxed = true)
        val prefs = mockk<SharedPreferences>()
        every { prefs.getString(any(), any()) } returns null
        every { prefs.getLong(any(), any()) } returns 0L
        every { prefs.edit() } returns editor
        return prefs
    }

    private fun mockContext(prefs: SharedPreferences): Context {
        val ctx = mockk<Context>()
        every { ctx.getSharedPreferences("rival_ad_stack_config", Context.MODE_PRIVATE) } returns prefs
        return ctx
    }

    @Test
    fun valid_signature_allows_config_when_not_in_test_mode() {
        // Generate Ed25519 keypair
        val kpg = KeyPairGenerator.getInstance("Ed25519")
        val kp = kpg.generateKeyPair()
        val pubKeyX509 = kp.public.encoded

        val configId = "cfg-1"
        val version = 1L
        val timestamp = 1234567890L
        val message = signingMessage(configId, version, timestamp)
        val sig = Signature.getInstance("Ed25519")
        sig.initSign(kp.private)
        sig.update(message)
        val signatureB64 = Base64.getEncoder().encodeToString(sig.sign())

        // Serve config with valid signature
        val body = baseConfigJson(configId, version, timestamp, signatureB64)
        server.enqueue(MockResponse().setResponseCode(200).setBody(body))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-1",
            testMode = false,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )
        val context = mockContext(mockPrefs())

        val mgr = ConfigManager(context, sdkCfg, client = null, configPublicKey = pubKeyX509)
        mgr.loadConfig()
        // Should have loaded placement from remote config
        val pl = mgr.getPlacementConfig("pl1")
        assertNotNull(pl)
    }

    @Test
    fun tampered_signature_rejects_config_when_not_in_test_mode() {
        val kpg = KeyPairGenerator.getInstance("Ed25519")
        val kp = kpg.generateKeyPair()
        val pubKeyX509 = kp.public.encoded

        val configId = "cfg-1"
        val version = 1L
        val timestamp = 1234567890L
        // Produce a signature for a different message to simulate tampering
        val wrongMessage = signingMessage(configId, version + 1, timestamp)
        val sig = Signature.getInstance("Ed25519")
        sig.initSign(kp.private)
        sig.update(wrongMessage)
        val signatureB64 = Base64.getEncoder().encodeToString(sig.sign())

        val body = baseConfigJson(configId, version, timestamp, signatureB64)
        server.enqueue(MockResponse().setResponseCode(200).setBody(body))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-1",
            testMode = false,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )
        val context = mockContext(mockPrefs())
        val mgr = ConfigManager(context, sdkCfg, client = null, configPublicKey = pubKeyX509)

        assertThrows(IllegalStateException::class.java) {
            mgr.loadConfig()
        }
        val pl = mgr.getPlacementConfig("pl1")
        assertNull(pl)
    }

    @Test
    fun test_mode_bypasses_signature_verification() {
        // Provide an invalid signature but with testMode=true, it should accept
        val body = baseConfigJson("cfg-2", 1L, 111L, signatureB64 = "invalid_base64_sig")
        server.enqueue(MockResponse().setResponseCode(200).setBody(body))
        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )
        val context = mockContext(mockPrefs())
        val mgr = ConfigManager(context, sdkCfg)
        mgr.loadConfig()
        val pl = mgr.getPlacementConfig("pl1")
        assertNotNull(pl)
    }

    @Test
    fun invalid_public_key_base64_rejects_config_in_production_builds() {
        val kpg = KeyPairGenerator.getInstance("Ed25519")
        val kp = kpg.generateKeyPair()

        val configId = "cfg-invalid-key"
        val version = 3L
        val timestamp = 987654321L
        val message = signingMessage(configId, version, timestamp)
        val sig = Signature.getInstance("Ed25519")
        sig.initSign(kp.private)
        sig.update(message)
        val signatureB64 = Base64.getEncoder().encodeToString(sig.sign())

        val body = baseConfigJson(configId, version, timestamp, signatureB64)
        server.enqueue(MockResponse().setResponseCode(200).setBody(body))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-2",
            testMode = false,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )
        val context = mockContext(mockPrefs())
        val invalidKeyBytes = ByteArray(0)
        val mgr = ConfigManager(context, sdkCfg, client = null, configPublicKey = invalidKeyBytes)

        assertThrows(IllegalStateException::class.java) {
            mgr.loadConfig()
        }
        val pl = mgr.getPlacementConfig("pl1")
        assertNull(pl)
    }

    @Test
    fun test_mode_bypasses_signature_even_with_invalid_public_key() {
        val kpg = KeyPairGenerator.getInstance("Ed25519")
        val kp = kpg.generateKeyPair()

        val configId = "cfg-invalid-key-test-mode"
        val version = 4L
        val timestamp = 111222333L
        val message = signingMessage(configId, version, timestamp)
        val sig = Signature.getInstance("Ed25519")
        sig.initSign(kp.private)
        sig.update(message)
        val signatureB64 = Base64.getEncoder().encodeToString(sig.sign())

        val body = baseConfigJson(configId, version, timestamp, signatureB64)
        server.enqueue(MockResponse().setResponseCode(200).setBody(body))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-3",
            testMode = true,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )
        val context = mockContext(mockPrefs())

        val invalidKeyBytes = ByteArray(0)
        val mgr = ConfigManager(context, sdkCfg, client = null, configPublicKey = invalidKeyBytes)
        mgr.loadConfig()
        val pl = mgr.getPlacementConfig("pl1")
        assertNotNull(pl)
    }
}
