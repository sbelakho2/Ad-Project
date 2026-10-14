package ee.apexmediation.sandbox.ctv

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Bundle
import android.util.Log
import android.view.KeyEvent
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity

class MainActivity : AppCompatActivity() {

    private val tag = "ApexCTV"

    private lateinit var initBtn: Button
    private lateinit var loadIntBtn: Button
    private lateinit var showIntBtn: Button
    private lateinit var loadRwdBtn: Button
    private lateinit var showRwdBtn: Button
    private lateinit var statusText: TextView
    private lateinit var overlay: View
    private lateinit var overlayText: TextView

    private var initialized = false
    private var interstitialLoaded = false
    private var rewardedLoaded = false
    private var presenting = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        initBtn = findViewById(R.id.btnInit)
        loadIntBtn = findViewById(R.id.btnLoadInterstitial)
        showIntBtn = findViewById(R.id.btnShowInterstitial)
        loadRwdBtn = findViewById(R.id.btnLoadRewarded)
        showRwdBtn = findViewById(R.id.btnShowRewarded)
        statusText = findViewById(R.id.txtStatus)
        overlay = findViewById(R.id.adOverlay)
        overlayText = findViewById(R.id.adOverlayText)

        initBtn.setOnClickListener { initialize() }
        loadIntBtn.setOnClickListener { loadInterstitial() }
        showIntBtn.setOnClickListener { showInterstitial() }
        loadRwdBtn.setOnClickListener { loadRewarded() }
        showRwdBtn.setOnClickListener { showRewarded() }

        updateButtons()
        log("app_start platform=android_tv")
    }

    override fun onResume() {
        super.onResume()
        log("lifecycle onResume platform=android_tv")
    }

    override fun onPause() {
        super.onPause()
        log("lifecycle onPause platform=android_tv")
    }

    private fun initialize() {
        if (initialized) {
            log("initialize already platform=android_tv")
            statusText.text = "Initialized"
            return
        }
        // Here we would call the real SDK with a TV-specific appId/config
        initialized = true
        statusText.text = "Initialized"
        log("initialize ok platform=android_tv")
        updateButtons()
    }

    private fun loadInterstitial() {
        if (!initialized) { toastStatus("Init first"); return }
        if (!isOnline()) {
            log("load_interstitial network=offline platform=android_tv")
            toastStatus("Offline: cannot load")
            return
        }
        interstitialLoaded = true
        log("load_interstitial ok platform=android_tv")
        updateButtons()
    }

    private fun showInterstitial() {
        if (!initialized || !interstitialLoaded || presenting) return
        presenting = true
        interstitialLoaded = false
        updateButtons()
        showAdOverlay("Interstitial (debug placeholder)\nplatform=android_tv")
        log("show_interstitial platform=android_tv")
    }

    private fun loadRewarded() {
        if (!initialized) { toastStatus("Init first"); return }
        if (!isOnline()) {
            log("load_rewarded network=offline platform=android_tv")
            toastStatus("Offline: cannot load")
            return
        }
        rewardedLoaded = true
        log("load_rewarded ok platform=android_tv")
        updateButtons()
    }

    private fun showRewarded() {
        if (!initialized || !rewardedLoaded || presenting) return
        presenting = true
        rewardedLoaded = false
        updateButtons()
        showAdOverlay("Rewarded (debug placeholder)\nplatform=android_tv\nReward: 1")
        log("show_rewarded platform=android_tv")
    }

    private fun showAdOverlay(text: String) {
        overlayText.text = text
        overlay.visibility = View.VISIBLE
        // Give initial focus to overlay to capture Back
        overlay.isFocusableInTouchMode = true
        overlay.requestFocus()
        // Dismiss automatically after short delay to simulate close
        overlay.postDelayed({ dismissOverlay() }, 800)
    }

    private fun dismissOverlay() {
        if (overlay.visibility == View.VISIBLE) {
            overlay.visibility = View.GONE
            presenting = false
            log("ad_closed platform=android_tv")
            updateButtons()
            // Return focus to main button row for remotes
            val container = findViewById<FrameLayout>(R.id.root)
            container.requestFocus()
        }
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // Back button dismisses overlay gracefully
        if (event.keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) {
            if (overlay.visibility == View.VISIBLE) {
                dismissOverlay()
                return true
            }
        }
        return super.dispatchKeyEvent(event)
    }

    private fun updateButtons() {
        initBtn.isEnabled = true
        loadIntBtn.isEnabled = initialized
        showIntBtn.isEnabled = initialized && interstitialLoaded && !presenting
        loadRwdBtn.isEnabled = initialized
        showRwdBtn.isEnabled = initialized && rewardedLoaded && !presenting
    }

    private fun toastStatus(msg: String) {
        statusText.text = msg
    }

    private fun log(message: String) {
        Log.d(tag, message)
    }

    private fun isOnline(): Boolean {
        val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val nw = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(nw) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
    }
}
