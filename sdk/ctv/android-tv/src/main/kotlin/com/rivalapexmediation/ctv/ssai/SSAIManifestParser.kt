package com.rivalapexmediation.ctv.ssai

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.URL
import java.util.regex.Pattern

/**
 * SSAI Manifest Parser for Android TV SDK
 *
 * Parses HLS and DASH manifests to extract SSAI ad cue information.
 */
class SSAIManifestParser(
    private val debug: Boolean = false
) {
    /**
     * Parsed ad cue from manifest
     */
    data class AdCue(
        val id: String,
        val startTime: Double,
        val duration: Double,
        val podId: String,
        val slotIndex: Int,
        val cueType: CueType,
        val metadata: Map<String, String>
    )

    enum class CueType(val value: String) {
        PREROLL("preroll"),
        MIDROLL("midroll"),
        POSTROLL("postroll")
    }

    /**
     * Parse result
     */
    data class ParseResult(
        val adCues: List<AdCue>,
        val totalDuration: Double?,
        val targetDuration: Double?,
        val isLive: Boolean,
        val errors: List<ParseError>
    )

    data class ParseError(
        val line: Int,
        val message: String
    )

    // Regex patterns
    private val targetDurationPattern = Pattern.compile("#EXT-X-TARGETDURATION:(\\d+)")
    private val extinfPattern = Pattern.compile("#EXTINF:([\\d.]+)")
    private val dateRangePattern = Pattern.compile("#EXT-X-DATERANGE:(.+)")
    private val cueOutPattern = Pattern.compile("#EXT-X-CUE-OUT(?::([\\d.]+))?")
    private val cueOutDurationPattern = Pattern.compile("DURATION=([\\d.]+)")
    private val attrPattern = Pattern.compile("([A-Z0-9-]+)=(\"[^\"]*\"|[^,]*)")

    /**
     * Parse HLS manifest for SSAI ad cues
     */
    fun parseHLS(manifest: String): ParseResult {
        val adCues = mutableListOf<AdCue>()
        val errors = mutableListOf<ParseError>()
        var totalDuration = 0.0
        var targetDuration: Double? = null
        var isLive = true
        var currentTime = 0.0
        var slotIndex = 0

        val lines = manifest.lines()

        for ((lineIndex, line) in lines.withIndex()) {
            val trimmed = line.trim()

            // Parse EXT-X-TARGETDURATION
            targetDurationPattern.matcher(trimmed).let { m ->
                if (m.find()) {
                    targetDuration = m.group(1).toDoubleOrNull()
                }
            }

            // Parse EXT-X-ENDLIST (VOD marker)
            if (trimmed == "#EXT-X-ENDLIST") {
                isLive = false
            }

            // Parse EXTINF for segment duration
            extinfPattern.matcher(trimmed).let { m ->
                if (m.find()) {
                    val duration = m.group(1).toDoubleOrNull() ?: 0.0
                    currentTime += duration
                    totalDuration += duration
                }
            }

            // Parse EXT-X-DATERANGE for SSAI cues
            dateRangePattern.matcher(trimmed).let { m ->
                if (m.find()) {
                    try {
                        parseDateRange(m.group(1), currentTime, slotIndex)?.let { cue ->
                            adCues.add(cue)
                            slotIndex++
                        }
                    } catch (e: Exception) {
                        errors.add(ParseError(lineIndex + 1, e.message ?: "Parse error"))
                    }
                }
            }

            // Parse EXT-X-CUE-OUT (legacy SCTE-35 signaling)
            cueOutPattern.matcher(trimmed).let { m ->
                if (m.find()) {
                    parseCueOut(trimmed, currentTime, slotIndex)?.let { cue ->
                        adCues.add(cue)
                        slotIndex++
                    }
                }
            }
        }

        return ParseResult(
            adCues = adCues,
            totalDuration = if (isLive) null else totalDuration,
            targetDuration = targetDuration,
            isLive = isLive,
            errors = errors
        )
    }

    /**
     * Parse HLS manifest from URL
     */
    suspend fun parseHLS(url: URL): ParseResult = withContext(Dispatchers.IO) {
        val manifest = BufferedReader(InputStreamReader(url.openStream())).use { it.readText() }
        parseHLS(manifest)
    }

    /**
     * Parse DASH MPD manifest for SSAI ad cues
     */
    fun parseDASH(manifest: String): ParseResult {
        val adCues = mutableListOf<AdCue>()
        val errors = mutableListOf<ParseError>()
        var totalDuration: Double? = null
        var isLive = true
        var slotIndex = 0

        // Check for live vs VOD
        if (manifest.contains("type=\"static\"")) {
            isLive = false
        }

        // Parse mediaPresentationDuration
        val durationPattern = Pattern.compile("mediaPresentationDuration=\"([^\"]+)\"")
        durationPattern.matcher(manifest).let { m ->
            if (m.find()) {
                totalDuration = parseISO8601Duration(m.group(1))
            }
        }

        // Parse EventStream events
        val eventStreamPattern = Pattern.compile(
            "<EventStream[^>]*schemeIdUri=\"urn:scte:scte35:2014:xml\\+bin\"[^>]*>([\\s\\S]*?)</EventStream>",
            Pattern.MULTILINE
        )
        val eventPattern = Pattern.compile("<Event[^>]*>([\\s\\S]*?)</Event>")

        eventStreamPattern.matcher(manifest).let { esm ->
            while (esm.find()) {
                val eventStreamContent = esm.group(1)
                eventPattern.matcher(eventStreamContent).let { em ->
                    while (em.find()) {
                        parseDASHEvent(em.group(0), slotIndex)?.let { cue ->
                            adCues.add(cue)
                            slotIndex++
                        }
                    }
                }
            }
        }

        return ParseResult(
            adCues = adCues,
            totalDuration = totalDuration,
            targetDuration = null,
            isLive = isLive,
            errors = errors
        )
    }

    /**
     * Parse DASH manifest from URL
     */
    suspend fun parseDASH(url: URL): ParseResult = withContext(Dispatchers.IO) {
        val manifest = BufferedReader(InputStreamReader(url.openStream())).use { it.readText() }
        parseDASH(manifest)
    }

    // Private helpers

    private fun parseDateRange(content: String, currentTime: Double, slotIndex: Int): AdCue? {
        val attributes = mutableMapOf<String, String>()

        attrPattern.matcher(content).let { m ->
            while (m.find()) {
                val key = m.group(1)
                var value = m.group(2)
                // Remove quotes
                if (value.startsWith("\"") && value.endsWith("\"")) {
                    value = value.substring(1, value.length - 1)
                }
                attributes[key] = value
            }
        }

        // Check for SSAI class
        val classAttr = attributes["CLASS"] ?: return null
        if (!classAttr.contains("ssai") && !classAttr.contains("ad")) {
            return null
        }

        val id = attributes["ID"] ?: return null
        val duration = attributes["DURATION"]?.toDoubleOrNull() ?: 0.0

        // Determine cue type
        val cueType = when {
            currentTime < 1 -> CueType.PREROLL
            classAttr.contains("post") -> CueType.POSTROLL
            else -> CueType.MIDROLL
        }

        val podId = attributes["X-POD-ID"] ?: id

        return AdCue(
            id = id,
            startTime = currentTime,
            duration = duration,
            podId = podId,
            slotIndex = slotIndex,
            cueType = cueType,
            metadata = attributes
        )
    }

    private fun parseCueOut(line: String, currentTime: Double, slotIndex: Int): AdCue? {
        var duration = 0.0

        // Parse duration from CUE-OUT tag
        if (line.contains(":")) {
            line.substringAfterLast(":").trim().toDoubleOrNull()?.let {
                duration = it
            }
        }

        // Also check for DURATION attribute
        cueOutDurationPattern.matcher(line).let { m ->
            if (m.find()) {
                duration = m.group(1).toDoubleOrNull() ?: 0.0
            }
        }

        val cueType = if (currentTime < 1) CueType.PREROLL else CueType.MIDROLL
        val id = "cue-out-$slotIndex"

        return AdCue(
            id = id,
            startTime = currentTime,
            duration = duration,
            podId = id,
            slotIndex = slotIndex,
            cueType = cueType,
            metadata = mapOf("source" to "EXT-X-CUE-OUT")
        )
    }

    private fun parseDASHEvent(eventXml: String, slotIndex: Int): AdCue? {
        val presentationTime = extractXmlAttribute(eventXml, "presentationTime")?.toDoubleOrNull() ?: 0.0
        val duration = extractXmlAttribute(eventXml, "duration")?.toDoubleOrNull() ?: 0.0
        val id = extractXmlAttribute(eventXml, "id") ?: "event-$slotIndex"

        val cueType = if (presentationTime < 1) CueType.PREROLL else CueType.MIDROLL

        return AdCue(
            id = id,
            startTime = presentationTime,
            duration = duration,
            podId = id,
            slotIndex = slotIndex,
            cueType = cueType,
            metadata = mapOf("source" to "DASH-EventStream")
        )
    }

    private fun extractXmlAttribute(xml: String, attribute: String): String? {
        val pattern = Pattern.compile("$attribute=\"([^\"]*)\"")
        val matcher = pattern.matcher(xml)
        return if (matcher.find()) matcher.group(1) else null
    }

    private fun parseISO8601Duration(duration: String): Double? {
        // Parse ISO 8601 duration (PT1H30M45S format)
        if (!duration.startsWith("PT")) return null
        
        var current = duration.substring(2)
        var totalSeconds = 0.0

        // Hours
        val hIndex = current.indexOf('H')
        if (hIndex != -1) {
            current.substring(0, hIndex).toDoubleOrNull()?.let {
                totalSeconds += it * 3600
            }
            current = current.substring(hIndex + 1)
        }

        // Minutes
        val mIndex = current.indexOf('M')
        if (mIndex != -1) {
            current.substring(0, mIndex).toDoubleOrNull()?.let {
                totalSeconds += it * 60
            }
            current = current.substring(mIndex + 1)
        }

        // Seconds
        val sIndex = current.indexOf('S')
        if (sIndex != -1) {
            current.substring(0, sIndex).toDoubleOrNull()?.let {
                totalSeconds += it
            }
        }

        return if (totalSeconds > 0) totalSeconds else null
    }
}
