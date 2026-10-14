package com.rivalapexmediation.sdk.debug

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import com.rivalapexmediation.sdk.BelAds
import com.rivalapexmediation.sdk.MediationSDK
import com.rivalapexmediation.sdk.logging.Logger
import com.rivalapexmediation.sdk.util.InstallId

/**
 * Minimal in-app Mediation Debugger panel.
 * Displays SDK state useful for integration debugging. No external deps; safe in release.
 */
object DebugPanel {
    @JvmStatic
    fun show(activity: Activity) {
        val sdk = try { MediationSDK.getInstance() } catch (t: Throwable) { null }
        val appId = try { sdk?.getAppId() ?: "" } catch (_: Throwable) { "" }
        val placements = try { sdk?.getPlacements()?.joinToString() ?: "" } catch (_: Throwable) { "" }
        val testMode = try { sdk?.isTestModeEffective() ?: false } catch (_: Throwable) { false }
        val consent = try { sdk?.getConsentDebugSummary() ?: emptyMap() } catch (_: Throwable) { emptyMap() }
        val installId = InstallId.get(activity)

        val builder = StringBuilder()
        builder.appendLine("Apex Mediation — Debug Panel")
        builder.appendLine("App ID: $appId")
        builder.appendLine("Install ID: $installId")
        builder.appendLine("Test Mode: $testMode")
        builder.appendLine("Placements: $placements")
        builder.appendLine("Consent: ${consent}")

        val textView = TextView(activity).apply {
            setTextIsSelectable(true)
            text = builder.toString()
            setPadding(32, 32, 32, 32)
        }
        val scroll = ScrollView(activity).apply { addView(textView) }

        AlertDialog.Builder(activity)
            .setTitle("Mediation Debugger")
            .setView(scroll)
            .setPositiveButton("Copy") { dlg, _ ->
                copyToClipboard(activity, textView.text.toString())
                Toast.makeText(activity, "Diagnostics copied", Toast.LENGTH_SHORT).show()
                dlg.dismiss()
            }
            .setNegativeButton("Close") { dlg, _ -> dlg.dismiss() }
            .show()
    }

    private fun copyToClipboard(ctx: Context, text: String) {
        val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("ApexMediation Diagnostics", text))
    }
}
