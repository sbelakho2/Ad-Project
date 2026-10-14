package com.rivalapexmediation.sdk.measurement

import android.content.Context
import android.view.View
import androidx.test.core.app.ApplicationProvider
import com.iab.omid.library.apex.TestOmidRecorder
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class OmSdkHelperTest {
    private lateinit var appContext: Context

    private fun setHelperField(name: String, value: Any?) {
        val field = OmSdkHelper::class.java.getDeclaredField(name)
        field.isAccessible = true
        field.set(OmSdkHelper, value)
    }

    @Before
    fun setup() {
        appContext = ApplicationProvider.getApplicationContext()
        TestOmidRecorder.reset()
        setHelperField("initialized", false)
        setHelperField("sdkAvailable", false)
        setHelperField("partner", null)
    }

    @After
    fun teardown() {
        TestOmidRecorder.reset()
        setHelperField("initialized", false)
        setHelperField("sdkAvailable", false)
        setHelperField("partner", null)
    }

    @Test
    fun startSession_returnsNull_whenOmSdkUnavailable() {
        // Simulate missing OMSDK classes and ensure helper no-ops safely.
        setHelperField("initialized", true)
        setHelperField("sdkAvailable", false)
        setHelperField("partner", null)

        val handle = OmSdkHelper.startSession(View(appContext), isVideo = false)

        assertEquals(null, handle)
    }

    @Test
    fun startAndFinishSession_withBundledOmid() {
        val rootView = View(appContext)
        val obstruction = View(appContext)

        assertTrue("OMSDK should initialize when bundled", OmSdkHelper.initIfAvailable(appContext))
        assertTrue("OMSDK availability should be true after init", OmSdkHelper.isAvailable())
        assertEquals(1, TestOmidRecorder.activated)

        val handle = OmSdkHelper.startSession(rootView, isVideo = false, friendlyObstructions = listOf(obstruction))
        assertNotNull("Expected session handle when OM SDK is bundled", handle)

        // Validate lifecycle and friendly obstruction bookkeeping from the stubs
        assertEquals(1, TestOmidRecorder.createdSessions.size)
        assertEquals(listOf(rootView), TestOmidRecorder.registeredViews)
        assertEquals(1, TestOmidRecorder.startedSessions.size)
        val obstructionList = TestOmidRecorder.friendlyObstructions[handle!!.adSession]
        assertEquals(listOf(obstruction), obstructionList)
        assertTrue("Loaded event should be fired", TestOmidRecorder.loadedEvents > 0)
        assertTrue("Impression event should be fired", TestOmidRecorder.impressionEvents > 0)

        OmSdkHelper.finishSession(handle)
        assertEquals(1, TestOmidRecorder.finishedSessions.size)
    }
}
