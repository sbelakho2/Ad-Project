package com.rivalapexmediation.ctv.mediation

import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

class NoFillTrackerTest {

    @Before
    fun setUp() {
        NoFillTracker.clear()
    }

    @After
    fun tearDown() {
        NoFillTracker.clear()
    }

    @Test
    fun `records no-fill events correctly`() {
        NoFillTracker.recordNoFill(
            sourceId = "admob",
            placementId = "banner_1",
            reason = NoFillReason.NO_INVENTORY,
            latencyMs = 150,
            adFormat = "banner"
        )

        val stats = NoFillTracker.getStats()
        assertEquals(1, stats.totalNoFills)
        assertEquals(150.0, stats.averageLatencyMs, 0.01)
        assertEquals(1, stats.topReasons[NoFillReason.NO_INVENTORY])
        assertEquals(1, stats.topSources["admob"])
        assertEquals(1, stats.topPlacements["banner_1"])
        assertEquals(1, stats.formatBreakdown["banner"])
    }

    @Test
    fun `tracks consecutive no-fills by source`() {
        repeat(5) {
            NoFillTracker.recordNoFill(
                sourceId = "failing_source",
                placementId = "placement_$it",
                reason = NoFillReason.TIMEOUT
            )
        }

        val stats = NoFillTracker.getStats()
        assertEquals(5, stats.topSources["failing_source"])
    }

    @Test
    fun `recordFill resets consecutive counter`() {
        repeat(3) {
            NoFillTracker.recordNoFill(
                sourceId = "source1",
                placementId = "p1",
                reason = NoFillReason.NO_INVENTORY
            )
        }

        NoFillTracker.recordFill("source1")

        NoFillTracker.recordNoFill(
            sourceId = "source1",
            placementId = "p2",
            reason = NoFillReason.NO_INVENTORY
        )

        // After reset, should only have 1 consecutive (not 4)
        val events = NoFillTracker.getRecentEvents(10)
        assertEquals(4, events.size)
    }

    @Test
    fun `getRecentEvents returns correct count`() {
        repeat(10) { i ->
            NoFillTracker.recordNoFill(
                sourceId = "source_$i",
                placementId = "placement",
                reason = NoFillReason.UNKNOWN
            )
        }

        val recent5 = NoFillTracker.getRecentEvents(5)
        assertEquals(5, recent5.size)

        val recent20 = NoFillTracker.getRecentEvents(20)
        assertEquals(10, recent20.size)
    }

    @Test
    fun `clear removes all data`() {
        repeat(5) {
            NoFillTracker.recordNoFill(
                sourceId = "source",
                placementId = "placement",
                reason = NoFillReason.TIMEOUT
            )
        }

        NoFillTracker.clear()

        val stats = NoFillTracker.getStats()
        assertEquals(0, stats.totalNoFills)
        assertTrue(stats.topReasons.isEmpty())
        assertTrue(stats.topSources.isEmpty())
    }

    @Test
    fun `tracks multiple reasons correctly`() {
        NoFillTracker.recordNoFill("s1", "p1", NoFillReason.TIMEOUT)
        NoFillTracker.recordNoFill("s2", "p2", NoFillReason.TIMEOUT)
        NoFillTracker.recordNoFill("s3", "p3", NoFillReason.NO_INVENTORY)
        NoFillTracker.recordNoFill("s4", "p4", NoFillReason.NETWORK_ERROR)

        val stats = NoFillTracker.getStats()
        assertEquals(4, stats.totalNoFills)
        assertEquals(2, stats.topReasons[NoFillReason.TIMEOUT])
        assertEquals(1, stats.topReasons[NoFillReason.NO_INVENTORY])
        assertEquals(1, stats.topReasons[NoFillReason.NETWORK_ERROR])
    }

    @Test
    fun `detects consecutive failure pattern`() {
        var patternDetected = false
        NoFillTracker.addPatternListener { pattern ->
            if (pattern.type == NoFillPattern.PatternType.CONSECUTIVE_FAILURES) {
                patternDetected = true
            }
        }

        repeat(5) {
            NoFillTracker.recordNoFill(
                sourceId = "bad_source",
                placementId = "placement_$it",
                reason = NoFillReason.SERVER_ERROR
            )
        }

        assertTrue("Should detect consecutive failure pattern", patternDetected)
    }

    @Test
    fun `getNoFillsByFormat returns correct breakdown`() {
        NoFillTracker.recordNoFill("s1", "p1", NoFillReason.NO_INVENTORY, adFormat = "banner")
        NoFillTracker.recordNoFill("s2", "p2", NoFillReason.NO_INVENTORY, adFormat = "banner")
        NoFillTracker.recordNoFill("s3", "p3", NoFillReason.NO_INVENTORY, adFormat = "interstitial")
        NoFillTracker.recordNoFill("s4", "p4", NoFillReason.NO_INVENTORY, adFormat = "rewarded")

        val byFormat = NoFillTracker.getNoFillsByFormat()
        assertEquals(2, byFormat["banner"])
        assertEquals(1, byFormat["interstitial"])
        assertEquals(1, byFormat["rewarded"])
    }

    @Test
    fun `NoFillReason fromString works correctly`() {
        assertEquals(NoFillReason.TIMEOUT, NoFillReason.fromString("timeout"))
        assertEquals(NoFillReason.NO_INVENTORY, NoFillReason.fromString("no_inventory"))
        assertEquals(NoFillReason.UNKNOWN, NoFillReason.fromString("invalid_reason"))
    }
}
