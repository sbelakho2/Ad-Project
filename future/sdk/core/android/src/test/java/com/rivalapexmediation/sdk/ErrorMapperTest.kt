package com.rivalapexmediation.sdk

import com.rivalapexmediation.sdk.contract.AdapterError
import com.rivalapexmediation.sdk.contract.ErrorCode
import com.rivalapexmediation.sdk.mapping.ErrorMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ErrorMapperTest {

    @Test
    fun moloco_maps_no_fill_and_timeout() {
        val nf = ErrorMapper.mapMolocoError(3002, "no fill")
        assertTrue(nf is AdapterError.Recoverable)
        assertEquals(ErrorCode.NO_FILL, nf.code)

        val to = ErrorMapper.mapMolocoError(3004, "timeout")
        assertTrue(to is AdapterError.Recoverable)
        assertEquals(ErrorCode.TIMEOUT, to.code)
    }

    @Test
    fun ironsource_maps_config_and_timeout() {
        val cfg = ErrorMapper.mapIronSourceError(509, "init failed")
        assertTrue(cfg is AdapterError.Fatal)
        assertEquals(ErrorCode.STATUS_4XX, cfg.code)

        val to = ErrorMapper.mapIronSourceError(1036, "timeout")
        assertTrue(to is AdapterError.Recoverable)
        assertEquals(ErrorCode.TIMEOUT, to.code)
    }

    @Test
    fun pangle_and_mintegral_map_expected() {
        val pTo = ErrorMapper.mapPangleError(40002, "timeout")
        assertTrue(pTo is AdapterError.Recoverable)
        assertEquals(ErrorCode.TIMEOUT, pTo.code)

        val mNf = ErrorMapper.mapMintegralError(-1, "no fill")
        assertTrue(mNf is AdapterError.Recoverable)
        assertEquals(ErrorCode.NO_FILL, mNf.code)
    }

    @Test
    fun vungle_tapjoy_smaato_mappings_cover_no_fill_network_timeout() {
        val vNoFill = ErrorMapper.mapVungleError(1, "no fill")
        assertTrue(vNoFill is AdapterError.Recoverable)
        assertEquals(ErrorCode.NO_FILL, vNoFill.code)

        val vNet = ErrorMapper.mapVungleError(2, "net err")
        assertTrue(vNet is AdapterError.Recoverable)
        assertEquals(ErrorCode.NETWORK_ERROR, vNet.code)

        val vTimeout = ErrorMapper.mapVungleError(10, "timeout")
        assertTrue(vTimeout is AdapterError.Recoverable)
        assertEquals(ErrorCode.TIMEOUT, vTimeout.code)

        val tNet = ErrorMapper.mapTapjoyError(-1001, "net err")
        assertTrue(tNet is AdapterError.Recoverable)
        assertEquals(ErrorCode.NETWORK_ERROR, tNet.code)

        val sNoFill = ErrorMapper.mapSmaatoError(204, "no ads")
        assertTrue(sNoFill is AdapterError.Recoverable)
        assertEquals(ErrorCode.NO_FILL, sNoFill.code)
    }
}
