package com.apex.sandbox.data

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import com.apex.sandbox.model.ConsentState

@RunWith(AndroidJUnit4::class)
class ConfigRepositoryInstrumentedTest {

    private lateinit var context: Context
    private lateinit var repo: ConfigRepository

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        repo = ConfigRepository(context)
    }

    @Test
    fun `loads sandbox_config json with expected defaults`() {
        val cfg = repo.loadConfig()
        assertEquals("https://api.apexmediation.ee/api/v1", cfg.apiBase)
        assertEquals("edcbe420-5e96-4c0f-b7ee-e5a10eb11db3", cfg.placements.interstitialA)
        assertEquals("92fe3cb3-8028-4a65-aacf-c9916b2efc29", cfg.placements.interstitialB)
        assertEquals("b8372490-f300-49e8-b7c2-a77fc04f85e9", cfg.placements.rewardedA)
        assertEquals("47998e9f-dfbe-4ba9-922c-3001f106722e", cfg.placements.bannerA)

        // Consent defaults from asset + prefs overlay (testMode default true from prefs)
        assertEquals(false, cfg.consent.gdpr)
        assertEquals(false, cfg.consent.ccpa)
        assertEquals(false, cfg.consent.coppa)
        assertEquals(true, cfg.consent.lat)
        assertEquals(true, cfg.consent.testMode)
    }

    @Test
    fun `persists and reloads consent via SharedPreferences`() {
        val consentSaved = ConsentState(
            gdpr = true,
            ccpa = true,
            coppa = true,
            lat = false,
            testMode = false,
        )
        repo.saveConsent(consentSaved)

        // Start from asset defaults and overlay prefs
        val loaded = repo.loadConsentDefaults(repo.loadConfig().consent)
        assertTrue(loaded.gdpr)
        assertTrue(loaded.ccpa)
        assertTrue(loaded.coppa)
        assertEquals(false, loaded.lat)
        assertEquals(false, loaded.testMode)
        assertNotNull(loaded)
    }
}
