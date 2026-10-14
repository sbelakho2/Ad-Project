package com.apex.sandbox.ui

import android.view.View
import android.widget.AdapterView
import androidx.test.core.app.ActivityScenario
import androidx.test.espresso.Espresso.onData
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.action.ViewActions.click
import androidx.test.espresso.assertion.ViewAssertions.matches
import androidx.test.espresso.matcher.ViewMatchers.*
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.apex.sandbox.MainActivity
import com.apex.sandbox.R
import org.hamcrest.CoreMatchers.anything
import org.hamcrest.CoreMatchers.containsString
import org.hamcrest.Matchers.anyOf
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityUiTests {

    @Test
    fun happyPath_interstitial_and_rewarded() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            // Initialize
            onView(withId(R.id.initBtn)).perform(click())
            Thread.sleep(400)
            onView(withId(R.id.statusText)).check(matches(withText(containsString("initialized"))))

            // Network A (default index 0)
            onView(withId(R.id.networkSpinner)).perform(click())
            onData(anything()).atPosition(0).perform(click())

            // Interstitial A load→show
            onView(withId(R.id.loadInterstitialBtn)).perform(click())
            Thread.sleep(350)
            onView(withId(R.id.statusText)).check(matches(withText(startsWith("Interstitial loaded"))))
            onView(withId(R.id.showInterstitialBtn)).perform(click())
            Thread.sleep(250)
            onView(withId(R.id.statusText)).check(matches(withText(startsWith("Interstitial shown"))))

            // Rewarded A load→show
            onView(withId(R.id.loadRewardedBtn)).perform(click())
            Thread.sleep(350)
            onView(withId(R.id.statusText)).check(matches(withText(startsWith("Rewarded loaded"))))
            onView(withId(R.id.showRewardedBtn)).perform(click())
            Thread.sleep(250)
            onView(withId(R.id.statusText)).check(matches(withText(startsWith("Rewarded shown"))))
        }
    }

    @Test
    fun errorStates_airplane_invalidPlacement_and_timeouts() {
        ActivityScenario.launch(MainActivity::class.java).use { _ ->
            // Initialize
            onView(withId(R.id.initBtn)).perform(click())
            Thread.sleep(400)

            // Airplane → network_unreachable
            onView(withId(R.id.airplaneToggle)).perform(click())
            onView(withId(R.id.loadInterstitialBtn)).perform(click())
            Thread.sleep(200)
            onView(withId(R.id.statusText)).check(matches(withText(containsString("network_unreachable"))))
            // Reset airplane
            onView(withId(R.id.airplaneToggle)).perform(click())

            // Invalid placement id
            onView(withId(R.id.invalidPlacementToggle)).perform(click())
            onView(withId(R.id.loadInterstitialBtn)).perform(click())
            Thread.sleep(200)
            onView(withId(R.id.statusText)).check(matches(withText(containsString("invalid_placement"))))
            onView(withId(R.id.invalidPlacementToggle)).perform(click())

            // FakeNetworkB sometimes no_fill — just ensure it doesn't crash and shows either loaded or no_fill
            onView(withId(R.id.networkSpinner)).perform(click())
            onData(anything()).atPosition(1).perform(click())
            onView(withId(R.id.loadInterstitialBtn)).perform(click())
            Thread.sleep(500)
            // Assert status contains either loaded or no_fill
            onView(withId(R.id.statusText)).check(matches(anyOf(withText(startsWith("Interstitial loaded")), withText(containsString("no_fill")))))

            // FakeNetworkC timeout (~3s)
            onView(withId(R.id.networkSpinner)).perform(click())
            onData(anything()).atPosition(2).perform(click())
            onView(withId(R.id.loadInterstitialBtn)).perform(click())
            Thread.sleep(3200)
            onView(withId(R.id.statusText)).check(matches(withText(containsString("timeout"))))
        }
    }

    @Test
    fun lifecycleStress_rotation_recreate_and_show_spam() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            // Initialize and prepare
            onView(withId(R.id.initBtn)).perform(click())
            Thread.sleep(400)
            onView(withId(R.id.networkSpinner)).perform(click())
            onData(anything()).atPosition(0).perform(click())
            onView(withId(R.id.loadInterstitialBtn)).perform(click())
            Thread.sleep(350)

            // Recreate to simulate rotation
            scenario.recreate()
            Thread.sleep(200)

            // Spam Show button multiple times; mutex should serialize to a single show
            repeat(5) { onView(withId(R.id.showInterstitialBtn)).perform(click()) }
            Thread.sleep(500)
            onView(withId(R.id.statusText)).check(matches(withText(startsWith("Interstitial shown"))))
        }
    }

    @Test
    fun consentPersistence_toggles_survive_relaunch() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            // Flip toggles
            onView(withId(R.id.gdprToggle)).perform(click())
            onView(withId(R.id.ccpaToggle)).perform(click())
            onView(withId(R.id.coppaToggle)).perform(click())
            onView(withId(R.id.latToggle)).perform(click())
            onView(withId(R.id.testModeToggle)).perform(click())
            Thread.sleep(150)

            // Recreate activity to simulate relaunch
            scenario.recreate()
            Thread.sleep(250)

            // Verify toggles remain in the chosen state
            onView(withId(R.id.gdprToggle)).check(matches(isChecked()))
            onView(withId(R.id.ccpaToggle)).check(matches(isChecked()))
            onView(withId(R.id.coppaToggle)).check(matches(isChecked()))
            onView(withId(R.id.latToggle)).check(matches(isNotChecked()))
            onView(withId(R.id.testModeToggle)).check(matches(isChecked()))
        }
    }

    @Test
    fun banner_start_stop_layout_stability() {
        ActivityScenario.launch(MainActivity::class.java).use { _ ->
            // Start banner
            onView(withId(R.id.bannerStartBtn)).perform(click())
            Thread.sleep(200)
            // Stop banner and ensure no crash
            onView(withId(R.id.bannerStopBtn)).perform(click())
            Thread.sleep(100)
            // Start again to ensure layout remains stable
            onView(withId(R.id.bannerStartBtn)).perform(click())
            Thread.sleep(200)
            // Basic sanity: status view still visible
            onView(withId(R.id.statusText)).check(matches(isDisplayed()))
        }
    }
}
