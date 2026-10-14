package com.rivalapexmediation.sdk

import com.rivalapexmediation.sdk.models.*
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

private class DummyAdapter(
    override val name: String = "dummy",
    override val version: String = "1.0.0"
) : AdAdapter {
    override fun isAvailable(): Boolean = true
    var destroyed = false
    override fun loadAd(placement: String, config: PlacementConfig): AdResponse? {
        return AdResponse(
            ad = Ad(id = "ad1", placementId = placement, networkName = name, adType = AdType.INTERSTITIAL, ecpm = 1.0,
                creative = Creative.Banner(320, 50, "<div></div>")),
            ecpm = 1.0,
            loadTime = 10,
            networkName = name
        )
    }
    override fun destroy() { destroyed = true }
}

class AdapterRegistryTest {
    @Test
    fun register_get_shutdown() {
        val reg = AdapterRegistry()
        val dummy = DummyAdapter()
        reg.register(dummy.name, dummy)
        val got = reg.getAdapter("dummy")
        assertNotNull(got)
        assertEquals("dummy", got!!.name)
        reg.shutdown()
        assertTrue(dummy.destroyed)
        // After shutdown, registry should be empty
        assertEquals(null, reg.getAdapter("dummy"))
    }
}
