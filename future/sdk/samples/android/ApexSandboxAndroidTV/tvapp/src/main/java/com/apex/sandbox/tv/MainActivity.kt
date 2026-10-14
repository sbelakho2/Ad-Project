package com.apex.sandbox.tv

import android.os.Bundle
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.appcompat.app.AlertDialog
import android.util.Log
import android.widget.Button
import android.widget.LinearLayout

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        if (!TvEmulatorGuard.isRunningOnEmulator()) {
            AlertDialog.Builder(this)
                .setTitle("Emulator Only")
                .setMessage("This TV sandbox app is restricted to Android TV Emulators. The app will now exit.")
                .setCancelable(false)
                .setPositiveButton("Exit") { _, _ -> finish() }
                .show()
            return
        }

        val cfg = TvSandboxConfigLoader.load(this)
        Log.i("ApexSandboxTV", "Loaded config: appId=${cfg.appId}, forceAdapterPipeline=${cfg.forceAdapterPipeline}, whitelist=${cfg.adapterWhitelist}")

        // Initialize and apply sandbox flags via reflection if core SDK is present
        ReflectionSdkBridge.initializeIfPresent(this, cfg.appId ?: "sandbox-tv")
        if (cfg.forceAdapterPipeline == true) {
            ReflectionSdkBridge.setSandboxForceAdapterPipeline(true)
        }
        val wl = cfg.adapterWhitelist ?: emptyList<String>()
        if (wl.isNotEmpty()) {
            ReflectionSdkBridge.setSandboxAdapterWhitelist(wl)
        }
        val names = ReflectionSdkBridge.getAdapterNames()
        Log.i("ApexSandboxTV", "Core adapter names detected: ${names.joinToString(",")}")

        val info = TextView(this).apply {
            text = "Apex Sandbox Android TV\n(Leanback stub UI)\nappId=${cfg.appId}\nwhitelist=${cfg.adapterWhitelist ?: listOf<String>()}\nforceAdapterPipeline=${cfg.forceAdapterPipeline == true}"
            textSize = 20f
            setPadding(24, 24, 24, 24)
        }
        val btn = Button(this).apply {
            text = "Run All (Interstitial)"
            setOnClickListener {
                ReflectionSdkBridge.runAllInterstitial("tv_interstitial")
            }
        }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24, 24, 24, 24)
            addView(info)
            addView(btn)
        }
        setContentView(root)
    }
}