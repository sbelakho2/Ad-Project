package com.rivalapexmediation.sdk.adaptive

import android.content.Context
import android.content.res.Configuration
import android.util.DisplayMetrics
import android.view.WindowManager
import kotlin.math.roundToInt

/**
 * Adaptive Banner Support
 * 
 * Calculates optimal banner sizes based on device characteristics,
 * matching Google AdMob's Adaptive Banner implementation.
 * 
 * Industry standard alignment:
 * - Google AdMob: AdSize.getCurrentOrientationAnchoredAdaptiveBannerAdSize()
 * - AppLovin MAX: MaxAdFormat.ADAPTIVE_BANNER
 */
object AdaptiveBanner {
    
    /**
     * Standard IAB banner sizes
     */
    object StandardSizes {
        val BANNER = AdSize(320, 50)
        val LARGE_BANNER = AdSize(320, 100)
        val MEDIUM_RECTANGLE = AdSize(300, 250)
        val FULL_BANNER = AdSize(468, 60)
        val LEADERBOARD = AdSize(728, 90)
        
        // Adaptive placeholders
        val ADAPTIVE = AdSize(-1, -1) // Indicates adaptive sizing
    }
    
    /**
     * Banner ad size
     */
    data class AdSize(
        val width: Int,
        val height: Int
    ) {
        val isAdaptive: Boolean get() = width == -1 || height == -1
        
        fun getWidthInPixels(context: Context): Int {
            if (isAdaptive) throw IllegalStateException("Adaptive size must be calculated first")
            return (width * context.resources.displayMetrics.density).roundToInt()
        }
        
        fun getHeightInPixels(context: Context): Int {
            if (isAdaptive) throw IllegalStateException("Adaptive size must be calculated first")
            return (height * context.resources.displayMetrics.density).roundToInt()
        }
        
        override fun toString(): String = "${width}x$height"
    }
    
    /**
     * Orientation for adaptive banners
     */
    enum class Orientation {
        PORTRAIT,
        LANDSCAPE,
        CURRENT
    }
    
    /**
     * Get adaptive banner size for current device orientation
     * 
     * @param context Application context
     * @param maxWidth Maximum width in dp (use -1 for full screen width)
     * @return Calculated AdSize for the current orientation
     */
    @JvmStatic
    @JvmOverloads
    fun getCurrentOrientationAdaptiveAdSize(
        context: Context,
        maxWidth: Int = -1
    ): AdSize {
        val orientation = if (context.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE) {
            Orientation.LANDSCAPE
        } else {
            Orientation.PORTRAIT
        }
        return getAdaptiveAdSize(context, maxWidth, orientation)
    }
    
    /**
     * Get adaptive banner size for portrait orientation
     */
    @JvmStatic
    @JvmOverloads
    fun getPortraitAdaptiveAdSize(
        context: Context,
        maxWidth: Int = -1
    ): AdSize = getAdaptiveAdSize(context, maxWidth, Orientation.PORTRAIT)
    
    /**
     * Get adaptive banner size for landscape orientation
     */
    @JvmStatic
    @JvmOverloads
    fun getLandscapeAdaptiveAdSize(
        context: Context,
        maxWidth: Int = -1
    ): AdSize = getAdaptiveAdSize(context, maxWidth, Orientation.LANDSCAPE)
    
    /**
     * Get inline adaptive banner size (for scrollable content)
     * 
     * @param context Application context
     * @param width Banner width in dp
     * @param maxHeight Maximum height in dp
     * @return Calculated AdSize
     */
    @JvmStatic
    fun getInlineAdaptiveAdSize(
        context: Context,
        width: Int,
        maxHeight: Int
    ): AdSize {
        val height = calculateInlineHeight(width, maxHeight)
        return AdSize(width, height)
    }
    
    /**
     * Calculate adaptive banner size
     */
    private fun getAdaptiveAdSize(
        context: Context,
        maxWidth: Int,
        orientation: Orientation
    ): AdSize {
        val displayMetrics = context.resources.displayMetrics
        val density = displayMetrics.density
        
        // Calculate available width in dp
        val screenWidthDp = when {
            maxWidth > 0 -> maxWidth
            else -> getScreenWidth(context, orientation)
        }
        
        // Calculate adaptive height based on width and orientation
        val height = calculateAdaptiveHeight(screenWidthDp, orientation)
        
        return AdSize(screenWidthDp, height)
    }
    
    /**
     * Get screen width in dp for the specified orientation
     */
    private fun getScreenWidth(context: Context, orientation: Orientation): Int {
        val displayMetrics = context.resources.displayMetrics
        val density = displayMetrics.density
        
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val display = windowManager.defaultDisplay
        val metrics = DisplayMetrics()
        display.getMetrics(metrics)
        
        val widthPx = when (orientation) {
            Orientation.PORTRAIT -> minOf(metrics.widthPixels, metrics.heightPixels)
            Orientation.LANDSCAPE -> maxOf(metrics.widthPixels, metrics.heightPixels)
            Orientation.CURRENT -> metrics.widthPixels
        }
        
        return (widthPx / density).roundToInt()
    }
    
    /**
     * Calculate adaptive banner height based on width
     * 
     * Height calculation follows Google's adaptive banner guidelines:
     * - Width <= 400dp: height = 50dp (standard banner)
     * - Width > 400dp && <= 720dp: height = 50-90dp (scaled)
     * - Width > 720dp: height = 90dp (leaderboard)
     */
    private fun calculateAdaptiveHeight(widthDp: Int, orientation: Orientation): Int {
        return when {
            widthDp <= 320 -> 50
            widthDp <= 400 -> 50
            widthDp <= 468 -> 60
            widthDp <= 720 -> {
                // Interpolate between 60 and 90
                val ratio = (widthDp - 468).toFloat() / (720 - 468)
                (60 + (ratio * 30)).roundToInt()
            }
            else -> 90
        }
    }
    
    /**
     * Calculate inline adaptive height
     */
    private fun calculateInlineHeight(widthDp: Int, maxHeightDp: Int): Int {
        val calculatedHeight = when {
            widthDp <= 320 -> 250 // Medium rectangle for narrow
            widthDp <= 468 -> 200
            widthDp <= 720 -> 150
            else -> 100
        }
        return minOf(calculatedHeight, maxHeightDp)
    }
    
    /**
     * Check if device is a tablet
     */
    @JvmStatic
    fun isTablet(context: Context): Boolean {
        val screenLayout = context.resources.configuration.screenLayout
        return (screenLayout and Configuration.SCREENLAYOUT_SIZE_MASK) >= Configuration.SCREENLAYOUT_SIZE_LARGE
    }
    
    /**
     * Get recommended banner type for device
     */
    @JvmStatic
    fun getRecommendedBannerType(context: Context): AdSize {
        return if (isTablet(context)) {
            StandardSizes.LEADERBOARD
        } else {
            getCurrentOrientationAdaptiveAdSize(context)
        }
    }
}
