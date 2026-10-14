package com.rivalapexmediation.ctv

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.junit.After
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * CTV intentionally ships without OMSDK; ensure initialization succeeds when OMSDK is absent.
 */
@RunWith(RobolectricTestRunner::class)
class OmSdkAbsentTest {
    private lateinit var appContext: Context

    @Before
    fun setup() {
        appContext = ApplicationProvider.getApplicationContext()
        resetApexMediation()
    }

    @After
    fun teardown() {
        resetApexMediation()
    }

    private fun resetApexMediation() {
        val cls = ApexMediation::class.java
        fun setField(name: String, value: Any?) {
            val f = cls.getDeclaredField(name)
            f.isAccessible = true
            f.set(ApexMediation, value)
        }
        setField("initialized", false)
        setField("configMgr", null)
    }

    @Test
    fun initialize_succeeds_whenOmsdkMissing() {
        assertThrows(ClassNotFoundException::class.java) {
            Class.forName("com.iab.omid.library.apex.Omid")
        }

        ApexMediation.initialize(
            appContext,
            SDKConfig(
                appId = "omsdk_missing_ctv",
                apiBaseUrl = "https://api.apexmediation.ee/api/v1",
                testMode = true,
            )
        )

        assertTrue(ApexMediation.isInitialized())
    }
}
