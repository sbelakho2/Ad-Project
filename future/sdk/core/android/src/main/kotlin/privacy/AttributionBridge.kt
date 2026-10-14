package com.rivalapexmediation.sdk.privacy

import android.content.Context
import android.net.Uri
import android.os.Build
import android.util.Log
import java.util.concurrent.Executor

/**
 * Thin wrapper over Android Privacy Sandbox Attribution Reporting APIs.
 * Uses reflection to avoid compile-time dependency on adservices classes.
 * Safe no-op on devices/ROMs without the AdServices module.
 */
object AttributionBridge {
    private const val TAG = "AttributionBridge"

    /** Whether the device/runtime exposes the AttributionManager APIs. */
    fun isAvailable(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
        ClassAvailability.attributionManager != null

    /** Best-effort source registration. Returns true when the call is dispatched, false otherwise. */
    fun registerSource(context: Context, sourceUri: Uri, debugKey: String? = null): Boolean {
        val manager = resolveManager(context) ?: return false
        return try {
            val request = SourceRequestFactory.build(sourceUri, debugKey) ?: return false
            val executor = MainExecutorProvider.mainExecutor(context)
            val receiver = OutcomeReceiverAdapters.successOnly {
                // best-effort; nothing else to do
            }
            manager.javaClass.getMethod(
                "registerSource",
                ClassAvailability.sourceRequest,
                Executor::class.java,
                ClassAvailability.outcomeReceiver
            ).invoke(manager, request, executor, receiver)
            true
        } catch (t: Throwable) {
            Log.d(TAG, "registerSource failed: ${t.message}")
            false
        }
    }

    /** Best-effort trigger registration. Returns true when the call is dispatched, false otherwise. */
    fun registerTrigger(context: Context, triggerUri: Uri, debugKey: String? = null): Boolean {
        val manager = resolveManager(context) ?: return false
        return try {
            val request = TriggerRequestFactory.build(triggerUri, debugKey) ?: return false
            val executor = MainExecutorProvider.mainExecutor(context)
            val receiver = OutcomeReceiverAdapters.successOnly { }
            manager.javaClass.getMethod(
                "registerTrigger",
                ClassAvailability.triggerRequest,
                Executor::class.java,
                ClassAvailability.outcomeReceiver
            ).invoke(manager, request, executor, receiver)
            true
        } catch (t: Throwable) {
            Log.d(TAG, "registerTrigger failed: ${t.message}")
            false
        }
    }

    private fun resolveManager(context: Context): Any? {
        if (!isAvailable()) return null
        val cls = ClassAvailability.attributionManager ?: return null
        return try {
            context.getSystemService(cls)
        } catch (_: Throwable) {
            null
        }
    }

    private object ClassAvailability {
        val attributionManager: Class<*>? = load("android.adservices.attribution.AttributionManager")
        val sourceRequest: Class<*>? = load("android.adservices.attribution.SourceRegistrationRequest")
        val triggerRequest: Class<*>? = load("android.adservices.attribution.TriggerRegistrationRequest")
        val outcomeReceiver: Class<*>? = load("android.os.OutcomeReceiver")

        private fun load(name: String): Class<*>? = try { Class.forName(name) } catch (_: Throwable) { null }
    }

    private object MainExecutorProvider {
        fun mainExecutor(context: Context): Executor {
            return try {
                context.mainExecutor
            } catch (_: Throwable) {
                Executor { it.run() }
            }
        }
    }

    private object SourceRequestFactory {
        fun build(uri: Uri, debugKey: String?): Any? {
            val reqClass = ClassAvailability.sourceRequest ?: return null
            val builderClass = reqClass.declaredClasses.firstOrNull { it.simpleName == "Builder" } ?: return null
            return try {
                val ctor = builderClass.getConstructor(Uri::class.java)
                val builder = ctor.newInstance(uri)
                if (!debugKey.isNullOrBlank()) {
                    runCatching {
                        builderClass.getMethod("setDebugKey", String::class.java).invoke(builder, debugKey)
                    }
                }
                builderClass.getMethod("build").invoke(builder)
            } catch (_: Throwable) {
                null
            }
        }
    }

    private object TriggerRequestFactory {
        fun build(uri: Uri, debugKey: String?): Any? {
            val reqClass = ClassAvailability.triggerRequest ?: return null
            val builderClass = reqClass.declaredClasses.firstOrNull { it.simpleName == "Builder" } ?: return null
            return try {
                val ctor = builderClass.getConstructor(Uri::class.java)
                val builder = ctor.newInstance(uri)
                if (!debugKey.isNullOrBlank()) {
                    runCatching {
                        builderClass.getMethod("setDebugKey", String::class.java).invoke(builder, debugKey)
                    }
                }
                builderClass.getMethod("build").invoke(builder)
            } catch (_: Throwable) {
                null
            }
        }
    }

    private object OutcomeReceiverAdapters {
        /** Builds a no-op OutcomeReceiver that ignores errors. */
        fun successOnly(onSuccess: () -> Unit): Any? {
            val iface = ClassAvailability.outcomeReceiver ?: return null
            return java.lang.reflect.Proxy.newProxyInstance(
                iface.classLoader,
                arrayOf(iface)
            ) { _, method, _ ->
                when (method.name) {
                    "onResult" -> onSuccess()
                }
                null
            }
        }
    }
}
