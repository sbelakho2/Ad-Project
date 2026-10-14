package com.apex.sandbox

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.widget.*
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AlertDialog
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.lifecycleScope
import com.apex.sandbox.sdk.FakeNetwork
import com.apex.sandbox.ui.SandboxViewModel
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private lateinit var vm: SandboxViewModel
    private var bannerHandler: Handler? = null
    private var bannerRunnable: Runnable? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        if (!EmulatorGuard.isRunningOnEmulator()) {
            AlertDialog.Builder(this)
                .setTitle("Emulator Only")
                .setMessage("This sandbox app is restricted to Android Emulators. It will now close.")
                .setCancelable(false)
                .setPositiveButton("Exit") { _, _ -> finish() }
                .show()
            return
        }

        enableEdgeToEdge()
        setContentView(R.layout.activity_main)
        ViewCompat.setOnApplyWindowInsetsListener(findViewById(R.id.root)) { v, insets ->
            val systemBars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            v.setPadding(systemBars.left, systemBars.top, systemBars.right, systemBars.bottom)
            insets
        }

        vm = ViewModelProvider(this)[SandboxViewModel::class.java]

        val status = findViewById<TextView>(R.id.statusText)
        val initBtn = findViewById<Button>(R.id.initBtn)
        val loadInterstitialBtn = findViewById<Button>(R.id.loadInterstitialBtn)
        val showInterstitialBtn = findViewById<Button>(R.id.showInterstitialBtn)
        val loadInterstitialBBtn = findViewById<Button>(R.id.loadInterstitialBBtn)
        val showInterstitialBBtn = findViewById<Button>(R.id.showInterstitialBBtn)
        val loadRewardedBtn = findViewById<Button>(R.id.loadRewardedBtn)
        val showRewardedBtn = findViewById<Button>(R.id.showRewardedBtn)
        val networkSpinner = findViewById<Spinner>(R.id.networkSpinner)
        val airplaneToggle = findViewById<CheckBox>(R.id.airplaneToggle)
        val invalidPlacementToggle = findViewById<CheckBox>(R.id.invalidPlacementToggle)
        val gdprToggle = findViewById<CheckBox>(R.id.gdprToggle)
        val ccpaToggle = findViewById<CheckBox>(R.id.ccpaToggle)
        val coppaToggle = findViewById<CheckBox>(R.id.coppaToggle)
        val latToggle = findViewById<CheckBox>(R.id.latToggle)
        val testModeToggle = findViewById<CheckBox>(R.id.testModeToggle)
        val clearLogBtn = findViewById<Button>(R.id.clearLogBtn)
        val logView = findViewById<TextView>(R.id.logView)
        val bannerContainer = findViewById<FrameLayout>(R.id.bannerContainer)

        // Spinner setup
        val adapter = ArrayAdapter(this, android.R.layout.simple_spinner_item, listOf("Network A (fill)", "Network B (no-fill)", "Network C (timeout)"))
        adapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        networkSpinner.adapter = adapter
        networkSpinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                val nw = when (position) {
                    0 -> FakeNetwork.A_ALWAYS_FILL
                    1 -> FakeNetwork.B_RANDOM_NO_FILL
                    else -> FakeNetwork.C_SLOW_TIMEOUT
                }
                vm.setNetwork(nw)
            }
            override fun onNothingSelected(parent: AdapterView<*>?) {}
        }

        airplaneToggle.setOnCheckedChangeListener { _, isChecked -> vm.setAirplane(isChecked) }
        invalidPlacementToggle.setOnCheckedChangeListener { _, isChecked -> vm.setInvalidPlacement(isChecked) }

        // Consent toggles will be initialized from config
        lifecycleScope.launch {
            vm.config.collectLatest { cfg ->
                cfg ?: return@collectLatest
                gdprToggle.isChecked = cfg.consent.gdpr
                ccpaToggle.isChecked = cfg.consent.ccpa
                coppaToggle.isChecked = cfg.consent.coppa
                latToggle.isChecked = cfg.consent.lat
                testModeToggle.isChecked = cfg.consent.testMode
            }
        }

        val consentListener = CompoundButton.OnCheckedChangeListener { _, _ ->
            val c = com.apex.sandbox.model.ConsentState(
                gdpr = gdprToggle.isChecked,
                ccpa = ccpaToggle.isChecked,
                coppa = coppaToggle.isChecked,
                lat = latToggle.isChecked,
                testMode = testModeToggle.isChecked,
            )
            vm.setConsent(c)
        }
        gdprToggle.setOnCheckedChangeListener(consentListener)
        ccpaToggle.setOnCheckedChangeListener(consentListener)
        coppaToggle.setOnCheckedChangeListener(consentListener)
        latToggle.setOnCheckedChangeListener(consentListener)
        testModeToggle.setOnCheckedChangeListener(consentListener)

        initBtn.setOnClickListener { vm.initialize() }

        lifecycleScope.launch {
            vm.status.collectLatest { status.text = it }
        }
        lifecycleScope.launch {
            vm.log.collectLatest { log ->
                logView.text = log.joinToString("\n")
            }
        }

        lifecycleScope.launch {
            vm.config.collectLatest { cfg ->
                cfg ?: return@collectLatest
                loadInterstitialBtn.setOnClickListener { vm.loadInterstitial(cfg.placements.interstitialA) }
                showInterstitialBtn.setOnClickListener { vm.showInterstitial(cfg.placements.interstitialA) }
                loadInterstitialBBtn.setOnClickListener { vm.loadInterstitial(cfg.placements.interstitialB) }
                showInterstitialBBtn.setOnClickListener { vm.showInterstitial(cfg.placements.interstitialB) }
                loadRewardedBtn.setOnClickListener { vm.loadRewarded(cfg.placements.rewardedA) }
                showRewardedBtn.setOnClickListener { vm.showRewarded(cfg.placements.rewardedA) }
            }
        }

        clearLogBtn.setOnClickListener { vm.clearLog() }

        // Banner placeholder that refreshes text every N seconds to test layout
        fun startBanner() {
            bannerHandler?.removeCallbacksAndMessages(null)
            bannerHandler = Handler(Looper.getMainLooper())
            val bannerText = TextView(this).apply {
                text = "[Banner] ${'$'}{System.currentTimeMillis() % 100000}"
                textAlignment = TextView.TEXT_ALIGNMENT_CENTER
            }
            bannerContainer.removeAllViews()
            bannerContainer.addView(bannerText)
            bannerRunnable = object : Runnable {
                override fun run() {
                    bannerText.text = "[Banner] refresh ${'$'}{System.currentTimeMillis() % 100000}"
                    bannerHandler?.postDelayed(this, 5000)
                }
            }
            bannerHandler?.postDelayed(bannerRunnable!!, 5000)
        }
        fun stopBanner() {
            bannerHandler?.removeCallbacksAndMessages(null)
            bannerContainer.removeAllViews()
        }
        findViewById<Button>(R.id.bannerStartBtn).setOnClickListener { startBanner() }
        findViewById<Button>(R.id.bannerStopBtn).setOnClickListener { stopBanner() }
    }

    override fun onDestroy() {
        super.onDestroy()
        bannerHandler?.removeCallbacksAndMessages(null)
    }
}
