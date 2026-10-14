package com.rivalapexmediation.ctv

/**
 * Configuration for the CTV/OTT SDK
 */
data class SDKConfig(
    val appId: String,
    val apiBaseUrl: String = "https://api.apexmediation.ee/api/v1",
    val apiKey: String? = null,
    val testMode: Boolean = false,
    val requestTimeoutMs: Int = 5000,
    // Optional Ed25519 public key (PEM) for OTA config signature verification
    val configPublicKeyPem: String? = null,
)
