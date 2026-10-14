package com.rivalapexmediation.sdk.telemetry

import androidx.test.core.app.ApplicationProvider
import com.rivalapexmediation.sdk.SDKConfig
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class TelemetryCollectorReservoirTest {

    private lateinit var collector: TelemetryCollector

    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<android.content.Context>()
        val cfg = SDKConfig(
            appId = "test-app",
            telemetryEnabled = true,
            observabilityEnabled = true,
            observabilitySampleRate = 1.0, // always sample for deterministic tests
            observabilityMaxQueue = 100
        )
        collector = TelemetryCollector(ctx, cfg)
        collector.start()
    }

    @Test
    fun reservoir_caps_at_200_and_drops_oldest() {
        val placement = "p-cap"
        val adapter = "net"

        // Push 300 monotonically increasing latencies.
        // Reservoir keeps last 200 values due to drop-oldest policy.
        for (i in 1..300) {
            collector.recordAdapterSpanFinish(
                traceId = "t$i",
                placement = placement,
                adapter = adapter,
                outcome = if (i % 2 == 0) "fill" else "no_fill",
                latencyMs = i.toLong(),
            )
        }

        // The retained reservoir should contain values [101..300].
        // With n = 200, index rule idx = floor(p * (n-1)).
        val pct = collector.getLocalPercentiles(placement, adapter)
        // p50 -> idx floor(0.5*(199))=99 -> value 101+99 = 200
        assertEquals(200L, pct["p50"])
        // p99 -> idx floor(0.99*(199))=197 -> value 101+197 = 298
        assertEquals(298L, pct["p99"])
    }
}
