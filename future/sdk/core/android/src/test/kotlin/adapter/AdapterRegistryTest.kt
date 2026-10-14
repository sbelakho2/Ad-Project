package com.rivalapexmediation.sdk.adapter

import androidx.test.core.app.ApplicationProvider
import com.rivalapexmediation.sdk.models.AdType
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
class AdapterRegistryTest {
    private val sandboxProp = "apex.sandbox.adapters"

    private fun <T> withSandboxDiscovery(block: () -> T): T {
        val previous = System.getProperty(sandboxProp)
        System.setProperty(sandboxProp, "1")
        return try {
            block()
        } finally {
            if (previous == null) {
                System.clearProperty(sandboxProp)
            } else {
                System.setProperty(sandboxProp, previous)
            }
        }
    }

    @Test
    fun discovery_and_registeredCount() {
        withSandboxDiscovery {
            val ctx = ApplicationProvider.getApplicationContext<android.content.Context>()
            val registry = AdapterRegistry(ctx, sdkVersion = "1.0.0")
            val available = registry.getAvailableAdapters()
            // Expect our three mock adapters to be discoverable
            assertTrue(available.contains("admob"))
            assertTrue(available.contains("applovin"))
            assertTrue(available.contains("unity"))
            assertTrue(registry.registeredCount >= 3)
        }
    }

    private fun configFor(network: String): Map<String, Any> = when (network) {
        "admob" -> mapOf("app_id" to "ca-app-pub-TEST")
        "applovin" -> mapOf("sdk_key" to "AL-TEST-KEY")
        "unity" -> mapOf("game_id" to "UNITY-TEST-GAME")
        else -> emptyMap()
    }

    @Test
    fun initialize_and_mock_load_all_adapters() {
        withSandboxDiscovery {
            val ctx = ApplicationProvider.getApplicationContext<android.content.Context>()
            val registry = AdapterRegistry(ctx, sdkVersion = "1.0.0")
            val targets = listOf("admob", "applovin", "unity")

            targets.forEach { name ->
                val adapter = registry.getAdapter(name)
                assertNotNull("$name adapter instance should be available", adapter)
                registry.initializeAdapter(name, configFor(name))
                assertTrue("$name should be initialized", registry.isInitialized(name))

                val latch = CountDownLatch(1)
                var loaded = false
                adapter!!.loadAd(
                    placement = "home",
                    adType = AdType.BANNER,
                    config = emptyMap(),
                    callback = object : AdLoadCallback {
                        override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                            loaded = true
                            latch.countDown()
                        }
                        override fun onAdFailed(error: String, errorCode: Int) {
                            latch.countDown()
                        }
                    }
                )
                assertTrue("$name mock should load an ad", latch.await(2, TimeUnit.SECONDS) && loaded)
            }

            // Diagnostics
            val report = registry.getInitializationReport()
            val byName = report.associateBy { it.networkName }
            targets.forEach { name ->
                val status = byName[name]
                assertNotNull(status)
                assertEquals(true, status!!.registered)
                assertEquals(true, status.initialized)
                assertEquals("1.0.0", status.version)
            }
        }
    }
}
