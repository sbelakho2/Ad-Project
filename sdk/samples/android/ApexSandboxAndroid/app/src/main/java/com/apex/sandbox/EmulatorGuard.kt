package com.apex.sandbox

import android.os.Build

object EmulatorGuard {
    fun isRunningOnEmulator(): Boolean {
        val brand = Build.BRAND
        val device = Build.DEVICE
        val product = Build.PRODUCT
        val hardware = Build.HARDWARE
        val model = Build.MODEL
        val fingerprint = Build.FINGERPRINT

        return (brand.startsWith("generic") || brand.startsWith("google")) ||
                device.startsWith("generic") ||
                product.contains("sdk") || product.contains("emulator") || product.contains("simulator") ||
                hardware.contains("goldfish") || hardware.contains("ranchu") || hardware.contains("qemu") ||
                model.contains("Android SDK built for") ||
                fingerprint.startsWith("generic") || fingerprint.startsWith("unknown")
    }
}