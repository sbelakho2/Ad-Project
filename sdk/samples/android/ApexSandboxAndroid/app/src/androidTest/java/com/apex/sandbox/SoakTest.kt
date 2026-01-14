package com.apex.sandbox

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Long-running sanity soak; duration controlled by instrumentation arg SOAK_MINUTES (default 2).
 * For a full 30-minute soak use the adb script at scripts/soak.sh.
 */
@RunWith(AndroidJUnit4::class)
@LargeTest
class SoakTest {

    @Test
    fun timedSoak_cyclePlacementsAndLifecycle() {
        val args = InstrumentationRegistry.getArguments()
        val minutes = args.getString("SOAK_MINUTES")?.toLongOrNull() ?: 2L
        val end = System.currentTimeMillis() + minutes * 60 * 1000L
        while (System.currentTimeMillis() < end) {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                // Initialize
                com.apex.sandbox.ui.TestHelpers.tapInit()
                // Cycle A/B/C networks quickly and load
                com.apex.sandbox.ui.TestHelpers.selectNetwork(0)
                com.apex.sandbox.ui.TestHelpers.tapLoadInterstitialA()
                Thread.sleep(350)
                com.apex.sandbox.ui.TestHelpers.tapShowInterstitialA()
                Thread.sleep(250)

                com.apex.sandbox.ui.TestHelpers.selectNetwork(1)
                com.apex.sandbox.ui.TestHelpers.tapLoadInterstitialA()
                Thread.sleep(500)

                com.apex.sandbox.ui.TestHelpers.selectNetwork(2)
                com.apex.sandbox.ui.TestHelpers.tapLoadInterstitialA()
                Thread.sleep(3200)

                // Rewarded cycle
                com.apex.sandbox.ui.TestHelpers.selectNetwork(0)
                com.apex.sandbox.ui.TestHelpers.tapLoadRewardedA()
                Thread.sleep(350)
                com.apex.sandbox.ui.TestHelpers.tapShowRewardedA()
                Thread.sleep(250)

                // Recreate to simulate rotation
                scenario.recreate()
            }
        }
    }
}
