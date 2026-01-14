package com.apex.sandbox.ui

import android.app.Application
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.apex.sandbox.model.ConsentState
import com.apex.sandbox.model.Placements
import com.apex.sandbox.model.SandboxConfig
import com.apex.sandbox.sdk.FakeNetwork
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SandboxViewModelHappyPathTest {
    private lateinit var app: Application

    @Before
    fun setup() {
        app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test
    fun happyPath_initialize_and_interstitial_rewarded_load_show_sequence() = runBlocking {
        val vm = SandboxViewModel(app)
        // Wait a bit for config to load in init
        delay(400)

        // Ensure we start uninitialized
        assertTrue(vm.status.value.contains("not initialized") || vm.status.value.contains("Config loaded"))

        // Force predictable consent and placements
        val placements = vm.config.value?.placements ?: Placements(
            interstitialA = "interstitial_a",
            interstitialB = "interstitial_b",
            rewardedA = "rewarded_a",
            bannerA = "banner_a",
        )
        vm.setConsent(ConsentState(gdpr = true, ccpa = false, coppa = false, lat = true, testMode = true))

        // Initialize
        vm.initialize()
        delay(400)
        assertTrue("SDK should be initialized", vm.status.value.contains("initialized"))

        // Network A always fills
        vm.setNetwork(FakeNetwork.A_ALWAYS_FILL)

        // Interstitial A load→show
        vm.loadInterstitial(placements.interstitialA)
        delay(300)
        assertTrue(vm.status.value.startsWith("Interstitial loaded"))
        vm.showInterstitial(placements.interstitialA)
        delay(250)
        assertTrue(vm.status.value.startsWith("Interstitial shown"))

        // Rewarded A load→show
        vm.loadRewarded(placements.rewardedA)
        delay(300)
        assertTrue(vm.status.value.startsWith("Rewarded loaded"))
        vm.showRewarded(placements.rewardedA)
        delay(250)
        assertTrue(vm.status.value.startsWith("Rewarded shown"))

        // Log should reflect ordered operations
        val log = vm.log.value.joinToString("\n")
        assertTrue(log.contains("Initialize requested"))
        assertTrue(log.contains("Load interstitial: ${placements.interstitialA}"))
        assertTrue(log.contains("Shown: ${placements.interstitialA}"))
        assertTrue(log.contains("Load rewarded: ${placements.rewardedA}"))
        assertTrue(log.contains("Shown: ${placements.rewardedA}"))
    }
}
