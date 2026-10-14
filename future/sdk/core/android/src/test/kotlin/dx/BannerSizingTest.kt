package com.rivalapexmediation.sdk.dx

import android.content.Context
import android.os.Looper
import android.view.View
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.test.core.app.ApplicationProvider
import com.rivalapexmediation.sdk.BelBanner
import com.rivalapexmediation.sdk.MediationSDK
import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.models.Creative
import io.mockk.every
import io.mockk.mockk
import io.mockk.mockkObject
import io.mockk.unmockkObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import java.util.concurrent.Executors

/**
 * Banner-specific regression tests ensuring adaptive sizing works and detach clears views off the UI thread.
 */
@RunWith(RobolectricTestRunner::class)
class BannerSizingTest {
    private lateinit var appContext: Context

    @Before
    fun setUp() {
        appContext = ApplicationProvider.getApplicationContext()
    }

    @Test
    fun attach_withCreative_scalesToContainerWidth() {
        mockkObject(MediationSDK.Companion)
        try {
            val sdk = mockk<MediationSDK>()
            val placementId = "banner.home"
            val creative = Creative.Banner(width = 320, height = 50, markupHtml = "<html></html>")
            val ad = Ad(
                id = "ad-banner-1",
                placementId = placementId,
                networkName = "mocknet",
                adType = AdType.BANNER,
                ecpm = 1.23,
                creative = creative
            )
            every { MediationSDK.getInstance() } returns sdk
            every { sdk.getCachedAd(placementId) } returns ad
            every { sdk.isTestModeEffective() } returns false

            val container = FrameLayout(appContext).apply {
                layoutParams = FrameLayout.LayoutParams(640, FrameLayout.LayoutParams.WRAP_CONTENT)
            }

            BelBanner.attach(container, placementId)

            assertEquals(1, container.childCount)
            val child = container.getChildAt(0)
            assertTrue(child is WebView)
            val frameParams = child.layoutParams as FrameLayout.LayoutParams
            assertEquals(100, frameParams.height)
        } finally {
            unmockkObject(MediationSDK.Companion)
        }
    }

    @Test
    fun detach_offMainThread_removesViews() {
        val container = FrameLayout(appContext).apply {
            addView(View(appContext))
        }

        val executor = Executors.newSingleThreadExecutor()
        try {
            executor.submit { BelBanner.detach(container) }.get()
        } finally {
            executor.shutdownNow()
        }

        // Flush posted removeAllViews() from background detach
        Shadows.shadowOf(Looper.getMainLooper()).idle()

        assertEquals(0, container.childCount)
    }

    @Test
    fun attach_backToBack_usesLatestCachedCreative() {
        mockkObject(MediationSDK.Companion)
        try {
            val sdk = mockk<MediationSDK>()
            val placementId = "banner.refresh"
            val creativeOne = Creative.Banner(width = 320, height = 50, markupHtml = "<html>first</html>")
            val creativeTwo = Creative.Banner(width = 320, height = 90, markupHtml = "<html>second</html>")
            val firstAd = Ad(
                id = "banner-1",
                placementId = placementId,
                networkName = "mocknet",
                adType = AdType.BANNER,
                ecpm = 1.0,
                creative = creativeOne
            )
            val secondAd = firstAd.copy(id = "banner-2", creative = creativeTwo)

            every { MediationSDK.getInstance() } returns sdk
            every { sdk.getCachedAd(placementId) } returns firstAd andThen secondAd andThen secondAd
            every { sdk.isTestModeEffective() } returns false

            val container = FrameLayout(appContext).apply {
                layoutParams = FrameLayout.LayoutParams(640, FrameLayout.LayoutParams.WRAP_CONTENT)
            }

            BelBanner.attach(container, placementId)
            assertEquals(1, container.childCount)
            val firstView = container.getChildAt(0) as WebView
            val firstHeight = (firstView.layoutParams as FrameLayout.LayoutParams).height
            assertEquals(100, firstHeight)

            BelBanner.attach(container, placementId)
            assertEquals(1, container.childCount)
            val secondView = container.getChildAt(0) as WebView
            assertTrue("Refresh should replace the rendered view", secondView !== firstView)
            val secondHeight = (secondView.layoutParams as FrameLayout.LayoutParams).height
            assertEquals(180, secondHeight)
        } finally {
            unmockkObject(MediationSDK.Companion)
        }
    }
}
