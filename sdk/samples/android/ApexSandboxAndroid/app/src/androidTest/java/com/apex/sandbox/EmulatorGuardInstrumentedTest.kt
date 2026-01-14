package com.apex.sandbox

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class EmulatorGuardInstrumentedTest {
    @Test
    fun `isRunningOnEmulator returns true on emulator`() {
        // This test should be executed on an emulator as per project policy.
        // If it runs on a physical device, the assertion may fail by design.
        val appContext = InstrumentationRegistry.getInstrumentation().targetContext
        // Sanity: ensure we loaded context
        assertTrue(appContext.packageName.startsWith("com.apex.sandbox.android"))

        assertTrue("This suite must run on an emulator", EmulatorGuard.isRunningOnEmulator())
    }
}
