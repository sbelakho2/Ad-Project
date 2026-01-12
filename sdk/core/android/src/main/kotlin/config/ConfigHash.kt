package com.rivalapexmediation.sdk.config

import java.security.MessageDigest

/**
 * SDK Configuration hash computation for cross-platform parity verification.
 * Uses SHA-256 with deterministic JSON serialization.
 * Hash format: "v1:<hex-digest>"
 *
 * This implementation matches the Web SDK's configHash.ts algorithm exactly
 * to ensure cross-platform configuration parity.
 */
object ConfigHash {

    /**
     * Configuration structure matching Web SDK's SDKConfig interface
     */
    data class SDKHashableConfig(
        val appId: String,
        val version: Int,
        val floors: Map<String, Double>,
        val caps: Map<String, CapConfig>,
        val pacing: PacingConfig,
        val networks: List<String>,
        val features: FeaturesConfig,
        val compliance: ComplianceConfig
    ) {
        data class CapConfig(
            val daily: Int,
            val hourly: Int
        )

        data class PacingConfig(
            val enabled: Boolean,
            val minIntervalMs: Int
        )

        data class FeaturesConfig(
            val bidding: Boolean,
            val waterfall: Boolean,
            val bannerRefresh: Boolean,
            val bannerRefreshIntervalMs: Int
        )

        data class ComplianceConfig(
            val gdprApplies: Boolean,
            val ccpaApplies: Boolean,
            val coppaApplies: Boolean
        )
    }

    /**
     * Compute deterministic SHA-256 hash of SDK configuration.
     * Uses sorted JSON serialization to ensure cross-platform parity with server.
     * Hash format: "v1:<hex-digest>"
     *
     * @param config SDK configuration object
     * @return Configuration hash string
     */
    @JvmStatic
    fun computeHash(config: SDKHashableConfig): String {
        val canonicalJson = buildCanonicalConfigJson(config)
        val hash = sha256(canonicalJson)
        return "v1:$hash"
    }

    /**
     * Validate that local config hash matches server hash.
     * Useful for debugging configuration sync issues.
     *
     * @param config Local SDK configuration
     * @param serverHash Hash returned from server
     * @return true if hashes match, false otherwise
     */
    @JvmStatic
    fun validateHash(config: SDKHashableConfig, serverHash: String): Boolean {
        val localHash = computeHash(config)
        return localHash == serverHash
    }

    /**
     * Create a default config for hash computation
     * when server config is not available.
     */
    @JvmStatic
    fun createDefaultConfig(appId: String): SDKHashableConfig {
        return SDKHashableConfig(
            appId = appId,
            version = 1,
            floors = emptyMap(),
            caps = emptyMap(),
            pacing = SDKHashableConfig.PacingConfig(enabled = false, minIntervalMs = 0),
            networks = emptyList(),
            features = SDKHashableConfig.FeaturesConfig(
                bidding = true,
                waterfall = true,
                bannerRefresh = false,
                bannerRefreshIntervalMs = 0
            ),
            compliance = SDKHashableConfig.ComplianceConfig(
                gdprApplies = false,
                ccpaApplies = false,
                coppaApplies = false
            )
        )
    }

    /**
     * Build canonical JSON string from config with sorted keys.
     * This ensures deterministic output for hash computation.
     * Matches Web SDK's buildCanonicalConfigJson() exactly.
     */
    private fun buildCanonicalConfigJson(config: SDKHashableConfig): String {
        val sb = StringBuilder()
        sb.append("{")

        // appId
        sb.append("\"appId\":\"${escapeJson(config.appId)}\",")

        // caps - sorted by placement ID
        sb.append("\"caps\":{")
        val sortedCaps = config.caps.toSortedMap()
        sortedCaps.entries.forEachIndexed { index, (key, cap) ->
            sb.append("\"${escapeJson(key)}\":{\"daily\":${cap.daily},\"hourly\":${cap.hourly}}")
            if (index < sortedCaps.size - 1) sb.append(",")
        }
        sb.append("},")

        // compliance - alphabetically sorted keys (ccpa, coppa, gdpr)
        sb.append("\"compliance\":{")
        sb.append("\"ccpaApplies\":${config.compliance.ccpaApplies},")
        sb.append("\"coppaApplies\":${config.compliance.coppaApplies},")
        sb.append("\"gdprApplies\":${config.compliance.gdprApplies}")
        sb.append("},")

        // features - alphabetically sorted keys
        sb.append("\"features\":{")
        sb.append("\"bannerRefresh\":${config.features.bannerRefresh},")
        sb.append("\"bannerRefreshIntervalMs\":${config.features.bannerRefreshIntervalMs},")
        sb.append("\"bidding\":${config.features.bidding},")
        sb.append("\"waterfall\":${config.features.waterfall}")
        sb.append("},")

        // floors - sorted by placement ID
        sb.append("\"floors\":{")
        val sortedFloors = config.floors.toSortedMap()
        sortedFloors.entries.forEachIndexed { index, (key, floor) ->
            sb.append("\"${escapeJson(key)}\":$floor")
            if (index < sortedFloors.size - 1) sb.append(",")
        }
        sb.append("},")

        // networks - sorted alphabetically
        sb.append("\"networks\":[")
        val sortedNetworks = config.networks.sorted()
        sortedNetworks.forEachIndexed { index, network ->
            sb.append("\"${escapeJson(network)}\"")
            if (index < sortedNetworks.size - 1) sb.append(",")
        }
        sb.append("],")

        // pacing
        sb.append("\"pacing\":{")
        sb.append("\"enabled\":${config.pacing.enabled},")
        sb.append("\"minIntervalMs\":${config.pacing.minIntervalMs}")
        sb.append("},")

        // version
        sb.append("\"version\":${config.version}")

        sb.append("}")
        return sb.toString()
    }

    /**
     * Escape JSON string characters
     */
    private fun escapeJson(string: String): String {
        val sb = StringBuilder()
        for (char in string) {
            when (char) {
                '"' -> sb.append("\\\"")
                '\\' -> sb.append("\\\\")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> sb.append(char)
            }
        }
        return sb.toString()
    }

    /**
     * Compute SHA-256 hash and return hex string
     */
    private fun sha256(message: String): String {
        val bytes = message.toByteArray(Charsets.UTF_8)
        val digest = MessageDigest.getInstance("SHA-256")
        val hashBytes = digest.digest(bytes)
        return hashBytes.joinToString("") { byte -> "%02x".format(byte) }
    }
}
