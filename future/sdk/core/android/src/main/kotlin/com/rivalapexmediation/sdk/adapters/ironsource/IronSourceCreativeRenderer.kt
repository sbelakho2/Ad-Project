package com.rivalapexmediation.sdk.adapters.ironsource

import android.app.Activity
import android.app.Dialog
import android.view.ViewGroup
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.widget.FrameLayout
import com.rivalapexmediation.sdk.contract.CloseReason
import com.rivalapexmediation.sdk.contract.PaidEvent
import com.rivalapexmediation.sdk.contract.RewardedCallbacks
import com.rivalapexmediation.sdk.contract.ShowCallbacks

/**
 * Simple WebView dialog renderer sufficient for mocked ironSource creatives in tests.
 * Production integration will swap this out for the vendor SDK-provided view layer.
 */
open class IronSourceCreativeRenderer {
    open fun showInterstitial(
        activity: Activity,
        markup: String,
        paidEvent: PaidEvent?,
        callbacks: ShowCallbacks
    ) {
        val dialog = createDialog(activity, markup) {
            callbacks.onClick()
        }
        dialog.setOnShowListener {
            callbacks.onImpression()
            paidEvent?.let(callbacks::onPaidEvent)
        }
        dialog.setOnDismissListener {
            callbacks.onClosed(CloseReason.DISMISSED)
        }
        dialog.show()
    }

    open fun showRewarded(
        activity: Activity,
        markup: String,
        paidEvent: PaidEvent?,
        callbacks: RewardedCallbacks,
        rewardType: String,
        rewardAmount: Double
    ) {
        val dialog = createDialog(activity, markup) {
            callbacks.onClick()
        }
        dialog.setOnShowListener {
            callbacks.onImpression()
            paidEvent?.let(callbacks::onPaidEvent)
        }
        dialog.setOnDismissListener {
            callbacks.onRewardVerified(rewardType, rewardAmount)
            callbacks.onClosed(CloseReason.COMPLETED)
        }
        dialog.show()
    }

    private fun createDialog(activity: Activity, markup: String, onClick: () -> Unit): Dialog {
        val webView = WebView(activity)
        webView.settings.javaScriptEnabled = true
        webView.webChromeClient = object : WebChromeClient() {}
        webView.loadDataWithBaseURL(null, markup, "text/html", "utf-8", null)
        webView.setOnClickListener { onClick() }

        val container = FrameLayout(activity)
        container.addView(
            webView,
            FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
        )

        return Dialog(activity).apply {
            setContentView(container)
        }
    }
}
