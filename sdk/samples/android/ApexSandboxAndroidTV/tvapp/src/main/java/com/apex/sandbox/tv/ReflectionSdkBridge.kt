package com.apex.sandbox.tv

import android.content.Context
import android.util.Log

/**
 * Best-effort reflection bridge to the core Android SDK without a hard dependency.
 * If the core SDK is present in the classpath, we can initialize in test mode and
 * apply sandbox flags (adapter whitelist, force adapter pipeline). Otherwise, calls
 * are no-ops and logs are emitted.
 */
object ReflectionSdkBridge {
    private const val TAG = "ApexSandboxTV"
    private const val SDK_CLASS = "com.rivalapexmediation.sdk.MediationSDK"
    private const val SDK_CONFIG = "com.rivalapexmediation.sdk.SDKConfig"
    private const val SDK_CONFIG_BUILDER = "com.rivalapexmediation.sdk.SDKConfig$Builder"

    private var sdkClass: Class<*>? = null
    private var sdkInstance: Any? = null

    private fun ensureSdkClass(): Class<*>? {
        if (sdkClass != null) return sdkClass
        return try {
            Class.forName(SDK_CLASS).also { sdkClass = it }
        } catch (t: Throwable) {
            Log.i(TAG, "Core SDK not on classpath: ${t.message}")
            null
        }
    }

    fun initializeIfPresent(context: Context, appId: String) {
        val clazz = ensureSdkClass() ?: return
        try {
            val cfgClazz = Class.forName(SDK_CONFIG)
            val builderClazz = Class.forName(SDK_CONFIG_BUILDER)
            val builder = builderClazz.getConstructor().newInstance()
            // builder.appId(appId).testMode(true)
            builderClazz.getMethod("appId", String::class.java).invoke(builder, appId)
            builderClazz.getMethod("testMode", Boolean::class.javaPrimitiveType).invoke(builder, true)
            val config = builderClazz.getMethod("build").invoke(builder)

            val init = clazz.getMethod("initialize", Context::class.java, String::class.java, cfgClazz)
            sdkInstance = init.invoke(null, context, appId, config)
            Log.i(TAG, "Initialized core SDK via reflection (testMode=true)")
        } catch (t: Throwable) {
            Log.w(TAG, "Failed to initialize core SDK via reflection: ${t.message}")
        }
    }

    fun setSandboxForceAdapterPipeline(enabled: Boolean): Boolean {
        val clazz = ensureSdkClass() ?: return false
        val inst = sdkInstance ?: return false
        return try {
            val m = clazz.getMethod("setSandboxForceAdapterPipeline", Boolean::class.javaPrimitiveType)
            m.invoke(inst, enabled)
            Log.i(TAG, "Sandbox force adapter pipeline: $enabled")
            true
        } catch (t: Throwable) {
            Log.w(TAG, "Failed to set forceAdapterPipeline: ${t.message}")
            false
        }
    }

    fun setSandboxAdapterWhitelist(names: List<String>): Boolean {
        val clazz = ensureSdkClass() ?: return false
        val inst = sdkInstance ?: return false
        return try {
            val m = clazz.getMethod("setSandboxAdapterWhitelist", List::class.java)
            m.invoke(inst, names)
            Log.i(TAG, "Sandbox adapter whitelist applied: ${names.joinToString(",")}")
            true
        } catch (t: Throwable) {
            Log.w(TAG, "Failed to set adapter whitelist: ${t.message}")
            false
        }
    }

    @Suppress("UNCHECKED_CAST")
    fun getAdapterNames(): List<String> {
        val clazz = ensureSdkClass() ?: return emptyList()
        val inst = sdkInstance ?: return emptyList()
        return try {
            val m = clazz.getMethod("getAdapterNames")
            (m.invoke(inst) as? List<*>)?.filterIsInstance<String>() ?: emptyList()
        } catch (t: Throwable) {
            Log.w(TAG, "Failed to get adapter names: ${t.message}")
            emptyList()
        }
    }

    fun runAllInterstitial(placement: String) {
        val clazz = ensureSdkClass() ?: return
        val inst = sdkInstance ?: return
        try {
            val cbInterface = Class.forName("com.rivalapexmediation.sdk.AdLoadCallback")
            val proxy = java.lang.reflect.Proxy.newProxyInstance(
                cbInterface.classLoader,
                arrayOf(cbInterface)
            ) { _, method, args ->
                when (method.name) {
                    "onAdLoaded" -> Log.i(TAG, "[runAll] loaded interstitial for placement=$placement")
                    "onError" -> {
                        val msg = if (args != null && args.size >= 2) args[1]?.toString() else "unknown"
                        Log.w(TAG, "[runAll] load error for placement=$placement: $msg")
                    }
                }
                null
            }
            val load = clazz.getMethod("loadAd", String::class.java, cbInterface)
            val names = getAdapterNames()
            for (n in names) {
                setSandboxAdapterWhitelist(listOf(n))
                Log.i(TAG, "[runAll] Trying adapter=$n")
                load.invoke(inst, placement, proxy)
            }
        } catch (t: Throwable) {
            Log.w(TAG, "Failed runAllInterstitial via reflection: ${t.message}")
        }
    }
}
