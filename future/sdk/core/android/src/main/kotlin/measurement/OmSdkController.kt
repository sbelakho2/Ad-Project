package com.rivalapexmediation.sdk.measurement

import android.app.Activity

/**
 * OM SDK controller scaffold for viewability/verification sessions.
 * - No external dependency required; host apps can provide a real implementation when bundling IAB OM SDK.
 * - Default NoOp implementation does nothing and is safe to ship.
 */
interface OmSdkController {
    /** Starts a display session for a shown ad. */
    fun startDisplaySession(activity: Activity, placementId: String, networkName: String, creativeType: String? = null)
    /** Starts a video session for a shown ad. */
    fun startVideoSession(activity: Activity, placementId: String, networkName: String, durationSec: Int? = null)
    /** Ends the current session for the placement if any. */
    fun endSession(placementId: String)
}

/** No-op default implementation that performs no measurement. */
class NoOpOmSdkController : OmSdkController {
    override fun startDisplaySession(activity: Activity, placementId: String, networkName: String, creativeType: String?) {}
    override fun startVideoSession(activity: Activity, placementId: String, networkName: String, durationSec: Int?) {}
    override fun endSession(placementId: String) {}
}
