package com.rivalapexmediation.sdk.runtime

import android.os.Looper
import androidx.activity.ComponentActivity
import com.rivalapexmediation.sdk.models.AdType
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows

@RunWith(RobolectricTestRunner::class)
class AdPresentationCoordinatorTest {

    @Before
    fun setUp() {
        AdPresentationCoordinator.resetForTesting()
    }

    @Test
    fun begin_runsImmediately_whenActivityAlreadyResumed() {
        val controller = Robolectric.buildActivity(TestActivity::class.java).setup()
        val activity = controller.get()
        var invoked = false

        val accepted = AdPresentationCoordinator.begin(activity, "placement", AdType.INTERSTITIAL) {
            invoked = true
            true
        }

        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(accepted)
        assertTrue(invoked)
    }

    @Test
    fun begin_waitsUntilActivityResumes() {
        val controller = Robolectric.buildActivity(TestActivity::class.java).create()
        val activity = controller.get()
        var invoked = false

        val accepted = AdPresentationCoordinator.begin(activity, "placement", AdType.INTERSTITIAL) {
            invoked = true
            true
        }

        assertTrue(accepted)
        assertFalse(invoked)

        controller.resume()
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(invoked)
    }

    @Test
    fun begin_rejectsConcurrentRequests() {
        val controller = Robolectric.buildActivity(TestActivity::class.java).create()
        val activity = controller.get()

        val first = AdPresentationCoordinator.begin(activity, "one", AdType.INTERSTITIAL) { true }
        val second = AdPresentationCoordinator.begin(activity, "two", AdType.REWARDED) { true }

        assertTrue(first)
        assertFalse(second)

        controller.resume()
        Shadows.shadowOf(Looper.getMainLooper()).idle()
    }

    private class TestActivity : ComponentActivity()
}
