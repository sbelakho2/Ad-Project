package com.rivalapexmediation.sdk.telemetry

import androidx.test.core.app.ApplicationProvider
import com.rivalapexmediation.sdk.SDKConfig
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class TelemetryCollectorTest {

    private lateinit var collector: TelemetryCollector

    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<android.content.Context>()
        val cfg = SDKConfig(
            appId = "test-app",
            telemetryEnabled = true,
            observabilityEnabled = true,
            observabilitySampleRate = 1.0, // ensure sampling always on for these tests
            observabilityMaxQueue = 100
        )
        collector = TelemetryCollector(ctx, cfg)
        collector.start()
    }

    @Test
    fun percentiles_areComputedFromRecordedLatencies() {
        val placement = "p1"
        val adapter = "admob"

        // Record 10 deterministic latencies
        val latencies = listOf(10L, 20L, 30L, 40L, 50L, 60L, 70L, 80L, 90L, 100L)
        latencies.forEach { l ->
            collector.recordAdapterSpanFinish(
                traceId = "t",
                placement = placement,
                adapter = adapter,
                outcome = "fill",
                latencyMs = l,
            )
        }

        val pct = collector.getLocalPercentiles(placement, adapter)
        // Using implementation’s index rule: idx = floor(p * (n-1))
        assertEquals(50L, pct["p50"])
        assertEquals(90L, pct["p95"])
        assertEquals(90L, pct["p99"])
    }

    @Test
    fun counters_increment_perOutcome() {
        val placement = "p2"
        val adapter = "unity"

        // Record outcomes
        collector.recordAdapterSpanFinish("t", placement, adapter, "fill", 12)
        collector.recordAdapterSpanFinish("t", placement, adapter, "fill", 8)
        collector.recordAdapterSpanFinish("t", placement, adapter, "no_fill", 5)
        collector.recordAdapterSpanFinish("t", placement, adapter, "timeout", 100)
        collector.recordAdapterSpanFinish("t", placement, adapter, "error", 7)

        val c = collector.getLocalCounters(placement, adapter)
        assertEquals(2L, c["fills"])
        assertEquals(1L, c["no_fills"])
        assertEquals(1L, c["timeouts"])
        assertEquals(1L, c["errors"])
    }

    @Test
    fun sampling_zero_disablesLocalMetrics() {
        val ctx = ApplicationProvider.getApplicationContext<android.content.Context>()
        val cfg = SDKConfig(
            appId = "test-app",
            telemetryEnabled = true,
            observabilityEnabled = true,
            observabilitySampleRate = 0.0,
            observabilityMaxQueue = 10
        )
        val local = TelemetryCollector(ctx, cfg)
        local.start()

        local.recordAdapterSpanFinish("t", "placement", "adapterX", "fill", 42)
        val pct = local.getLocalPercentiles("placement", "adapterX")
        val c = local.getLocalCounters("placement", "adapterX")
        assertEquals(0, pct.size)
        assertEquals(0, c.size)
    }
}

@RunWith(RobolectricTestRunner::class)
class TelemetryCollectorAdditionalTest {

    private lateinit var collector: TelemetryCollector

    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<android.content.Context>()
        val cfg = SDKConfig(
            appId = "test-app",
            telemetryEnabled = true,
            observabilityEnabled = true,
            observabilitySampleRate = 1.0, // ensure sampling always on for these tests
            observabilityMaxQueue = 100
        )
        collector = TelemetryCollector(ctx, cfg)
        collector.start()
    }

    @Test
    fun finish_without_start_still_updates_local_counters_and_percentiles() {
        val placement = "p-seq"
        val adapter = "applovin"

        // Intentionally emit only FINISH events (no START) — should still contribute to local metrics
        collector.recordAdapterSpanFinish("t1", placement, adapter, "fill", 42)
        collector.recordAdapterSpanFinish("t2", placement, adapter, "no_fill", 84)

        val counters = collector.getLocalCounters(placement, adapter)
        val pct = collector.getLocalPercentiles(placement, adapter)

        // We should have counts for fill and no_fill and percentiles computed over [42, 84]
        org.junit.Assert.assertEquals(1L, counters["fills"])
        org.junit.Assert.assertEquals(1L, counters["no_fills"])
        org.junit.Assert.assertEquals(42L, pct["p50"]) // floor index rule over 2 items -> first item
        org.junit.Assert.assertEquals(42L, pct["p95"]) // floor rule clamps to first sample with 2 values
    }
}
