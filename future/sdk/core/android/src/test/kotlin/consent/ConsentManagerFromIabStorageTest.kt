package com.rivalapexmediation.sdk.consent

import android.content.Context
import android.content.SharedPreferences
import io.mockk.every
import io.mockk.mockk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ConsentManagerFromIabStorageTest {
    @Test
    fun reads_from_IAB_preferences_primary() {
        val prefsPrimary = mockk<SharedPreferences>()
        val prefsAlt = mockk<SharedPreferences>()
        every { prefsPrimary.getString("IABTCF_TCString", null) } returns "TCF_TEST"
        every { prefsPrimary.contains("IABTCF_gdprApplies") } returns true
        every { prefsPrimary.getInt("IABTCF_gdprApplies", any()) } returns 1
        every { prefsPrimary.getString("IABUSPrivacy_String", null) } returns "1YNN"
        every { prefsAlt.getString(any(), any()) } returns null
        every { prefsAlt.contains(any()) } returns false
        every { prefsAlt.getInt(any(), any()) } returns -1

        val ctx = mockk<Context>()
        every { ctx.packageName } returns "com.example.app"
        every { ctx.getSharedPreferences("IABTCF", Context.MODE_PRIVATE) } returns prefsPrimary
        every { ctx.getSharedPreferences("com.example.app_preferences", Context.MODE_PRIVATE) } returns prefsAlt

        val state = ConsentManager.fromIabStorage(ctx)
        assertEquals(true, state.gdprApplies)
        assertEquals("TCF_TEST", state.consentString)
        assertEquals("1YNN", state.usPrivacy)
    }

    @Test
    fun falls_back_to_alt_prefs_and_handles_missing_values() {
        val prefsPrimary = mockk<SharedPreferences>()
        val prefsAlt = mockk<SharedPreferences>()
        every { prefsPrimary.getString(any(), any()) } returns null
        every { prefsPrimary.contains(any()) } returns false
        every { prefsPrimary.getInt(any(), any()) } returns -1
        every { prefsAlt.getString("IABTCF_TCString", null) } returns null
        every { prefsAlt.contains("IABTCF_gdprApplies") } returns true
        every { prefsAlt.getInt("IABTCF_gdprApplies", any()) } returns 0
        every { prefsAlt.getString("IABUSPrivacy_String", null) } returns null

        val ctx = mockk<Context>()
        every { ctx.packageName } returns "com.example.app"
        every { ctx.getSharedPreferences("IABTCF", Context.MODE_PRIVATE) } returns prefsPrimary
        every { ctx.getSharedPreferences("com.example.app_preferences", Context.MODE_PRIVATE) } returns prefsAlt

        val state = ConsentManager.fromIabStorage(ctx)
        assertEquals(false, state.gdprApplies)
        assertNull(state.consentString)
        assertNull(state.usPrivacy)
    }
}
