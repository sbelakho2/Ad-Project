package com.rivalapexmediation.sdk.util

import android.content.Context
import android.content.SharedPreferences
import io.mockk.every
import io.mockk.mockk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RolloutTest {
    private fun mockContextWithInstallId(id: String): Context {
        val prefs = mockk<SharedPreferences>()
        every { prefs.getString("install_id", any()) } returns id
        val editor = mockk<SharedPreferences.Editor>(relaxed = true)
        every { prefs.edit() } returns editor
        val ctx = mockk<Context>()
        every { ctx.getSharedPreferences("rival_ad_stack_install", Context.MODE_PRIVATE) } returns prefs
        return ctx
    }

    @Test
    fun bucket_isDeterministicForSameInstall() {
        val ctx = mockContextWithInstallId("install-1234")
        val b1 = Rollout.bucket(ctx)
        val b2 = Rollout.bucket(ctx)
        assertEquals(b1, b2)
        assertTrue(b1 in 0..99)
    }

    @Test
    fun isInRollout_respectsPercentageBounds() {
        val ctx = mockContextWithInstallId("install-5678")
        // 0% never includes
        assertEquals(false, Rollout.isInRollout(ctx, 0))
        // 100% always includes
        assertEquals(true, Rollout.isInRollout(ctx, 100))
        // Negative coerced to 0; >100 coerced to 100
        assertEquals(false, Rollout.isInRollout(ctx, -10))
        assertEquals(true, Rollout.isInRollout(ctx, 1000))
    }
}
