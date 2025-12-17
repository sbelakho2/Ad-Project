import Foundation

/// SSAI Manifest Parser for tvOS SDK
///
/// Parses HLS and DASH manifests to extract SSAI ad cue information.
public final class SSAIManifestParser {
    
    // MARK: - Types
    
    /// Parsed ad cue from manifest
    public struct AdCue: Codable, Equatable {
        public let id: String
        public let startTime: TimeInterval
        public let duration: TimeInterval
        public let podId: String
        public let slotIndex: Int
        public let cueType: CueType
        public let metadata: [String: String]
        
        public enum CueType: String, Codable {
            case preroll
            case midroll
            case postroll
        }
    }
    
    /// Parsed manifest result
    public struct ParseResult {
        public let adCues: [AdCue]
        public let totalDuration: TimeInterval?
        public let targetDuration: TimeInterval?
        public let isLive: Bool
        public let errors: [ParseError]
    }
    
    /// Parse error
    public struct ParseError: Error {
        public let line: Int
        public let message: String
    }
    
    // MARK: - Properties
    
    private let debug: Bool
    
    // MARK: - Initialization
    
    public init(debug: Bool = false) {
        self.debug = debug
    }
    
    // MARK: - HLS Parsing
    
    /// Parse HLS manifest for SSAI ad cues
    public func parseHLS(manifest: String) -> ParseResult {
        var adCues: [AdCue] = []
        var errors: [ParseError] = []

        let firstNonEmptyLine = manifest
            .components(separatedBy: .newlines)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
        if firstNonEmptyLine != "#EXTM3U" {
            errors.append(ParseError(line: 0, message: "Missing or invalid #EXTM3U header"))
        }

        var totalDuration: TimeInterval = 0
        var targetDuration: TimeInterval?
        var isLive = true
        var currentTime: TimeInterval = 0
        var slotIndex = 0
        
        let lines = manifest.components(separatedBy: .newlines)
        
        for (lineIndex, line) in lines.enumerated() {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            
            // Parse EXT-X-TARGETDURATION
            if trimmed.hasPrefix("#EXT-X-TARGETDURATION:") {
                if let value = extractValue(from: trimmed, prefix: "#EXT-X-TARGETDURATION:"),
                   let duration = TimeInterval(value) {
                    targetDuration = duration
                }
            }
            
            // Parse EXT-X-ENDLIST (VOD marker)
            if trimmed == "#EXT-X-ENDLIST" {
                isLive = false
            }
            
            // Parse EXTINF for segment duration
            if trimmed.hasPrefix("#EXTINF:") {
                if let value = extractValue(from: trimmed, prefix: "#EXTINF:")?.split(separator: ",").first,
                   let duration = TimeInterval(value) {
                    currentTime += duration
                    totalDuration += duration
                }
            }
            
            // Parse EXT-X-DATERANGE for SSAI cues
            if trimmed.hasPrefix("#EXT-X-DATERANGE:") {
                do {
                    if let cue = try parseDateRange(trimmed, at: currentTime, slotIndex: &slotIndex) {
                        adCues.append(cue)
                    }
                } catch let error as ParseError {
                    errors.append(error)
                }
            }
            
            // Parse EXT-X-CUE-OUT (legacy SCTE-35 signaling)
            if trimmed.hasPrefix("#EXT-X-CUE-OUT") {
                if let cue = parseCueOut(trimmed, at: currentTime, slotIndex: &slotIndex) {
                    adCues.append(cue)
                }
            }
        }
        
        return ParseResult(
            adCues: adCues,
            totalDuration: isLive ? nil : totalDuration,
            targetDuration: targetDuration,
            isLive: isLive,
            errors: errors
        )
    }
    
    /// Parse HLS manifest from URL
    public func parseHLS(url: URL, completion: @escaping (Result<ParseResult, Error>) -> Void) {
        URLSession.shared.dataTask(with: url) { [weak self] data, response, error in
            if let error = error {
                DispatchQueue.main.async { completion(.failure(error)) }
                return
            }
            
            guard let data = data, let manifest = String(data: data, encoding: .utf8) else {
                let error = ParseError(line: 0, message: "Failed to decode manifest")
                DispatchQueue.main.async { completion(.failure(error)) }
                return
            }
            
            guard let self = self else { return }
            let result = self.parseHLS(manifest: manifest)
            DispatchQueue.main.async { completion(.success(result)) }
        }.resume()
    }
    
    /// Parse HLS manifest from URL (async)
    @available(tvOS 13.0, *)
    public func parseHLS(url: URL) async throws -> ParseResult {
        let (data, _) = try await URLSession.shared.data(from: url)
        guard let manifest = String(data: data, encoding: .utf8) else {
            throw ParseError(line: 0, message: "Failed to decode manifest")
        }
        return parseHLS(manifest: manifest)
    }
    
    // MARK: - DASH Parsing
    
    /// Parse DASH MPD manifest for SSAI ad cues
    public func parseDASH(manifest: String) -> ParseResult {
        var adCues: [AdCue] = []
        var errors: [ParseError] = []
        var totalDuration: TimeInterval?
        var isLive = true
        var slotIndex = 0

        if !manifest.contains("<MPD") && !manifest.contains("<mpd") {
            errors.append(ParseError(line: 0, message: "Missing <MPD> root element"))
        }
        
        // Parse XML
        guard let data = manifest.data(using: .utf8) else {
            return ParseResult(
                adCues: [],
                totalDuration: nil,
                targetDuration: nil,
                isLive: true,
                errors: [ParseError(line: 0, message: "Failed to encode manifest")]
            )
        }
        
        // Simple regex-based parsing for key elements
        // In production, use XMLParser for proper parsing
        
        // Check for live vs VOD
        if manifest.contains("type=\"static\"") {
            isLive = false
        }
        
        // Parse mediaPresentationDuration
        if let durationMatch = extractAttribute(from: manifest, element: "MPD", attribute: "mediaPresentationDuration") {
            totalDuration = parseISO8601Duration(durationMatch)
        }
        
        // Parse EventStream events
        let eventPattern = #"<Event[^>]*>([\s\S]*?)</Event>"#
        let eventStreamPattern = #"<EventStream[^>]*schemeIdUri="urn:scte:scte35:2014:xml\+bin"[^>]*>([\s\S]*?)</EventStream>"#
        
        if let eventStreamRegex = try? NSRegularExpression(pattern: eventStreamPattern, options: []),
           let eventStreamMatch = eventStreamRegex.firstMatch(
               in: manifest,
               range: NSRange(manifest.startIndex..., in: manifest)
           ) {
            let eventStreamContent = String(manifest[Range(eventStreamMatch.range(at: 1), in: manifest)!])
            
            if let eventRegex = try? NSRegularExpression(pattern: eventPattern, options: []) {
                let matches = eventRegex.matches(
                    in: eventStreamContent,
                    range: NSRange(eventStreamContent.startIndex..., in: eventStreamContent)
                )
                
                for match in matches {
                    if let cue = parseDASHEvent(eventStreamContent, match: match, slotIndex: &slotIndex) {
                        adCues.append(cue)
                    }
                }
            }
        }
        
        return ParseResult(
            adCues: adCues,
            totalDuration: totalDuration,
            targetDuration: nil,
            isLive: isLive,
            errors: errors
        )
    }
    
    /// Parse DASH manifest from URL
    public func parseDASH(url: URL, completion: @escaping (Result<ParseResult, Error>) -> Void) {
        URLSession.shared.dataTask(with: url) { [weak self] data, response, error in
            if let error = error {
                DispatchQueue.main.async { completion(.failure(error)) }
                return
            }
            
            guard let data = data, let manifest = String(data: data, encoding: .utf8) else {
                let error = ParseError(line: 0, message: "Failed to decode manifest")
                DispatchQueue.main.async { completion(.failure(error)) }
                return
            }
            
            guard let self = self else { return }
            let result = self.parseDASH(manifest: manifest)
            DispatchQueue.main.async { completion(.success(result)) }
        }.resume()
    }
    
    // MARK: - Private Helpers
    
    private func extractValue(from line: String, prefix: String) -> String? {
        guard line.hasPrefix(prefix) else { return nil }
        return String(line.dropFirst(prefix.count))
    }
    
    private func parseDateRange(_ line: String, at currentTime: TimeInterval, slotIndex: inout Int) throws -> AdCue? {
        var attributes: [String: String] = [:]
        
        // Parse key-value pairs from DATERANGE tag
        let content = String(line.dropFirst("#EXT-X-DATERANGE:".count))
        let pattern = #"([A-Z0-9-]+)=("[^"]*"|[^,]*)"#
        
        guard let regex = try? NSRegularExpression(pattern: pattern, options: []) else {
            return nil
        }
        
        let matches = regex.matches(in: content, range: NSRange(content.startIndex..., in: content))
        
        for match in matches {
            if let keyRange = Range(match.range(at: 1), in: content),
               let valueRange = Range(match.range(at: 2), in: content) {
                let key = String(content[keyRange])
                var value = String(content[valueRange])
                
                // Remove quotes
                if value.hasPrefix("\"") && value.hasSuffix("\"") {
                    value = String(value.dropFirst().dropLast())
                }
                
                attributes[key] = value
            }
        }
        
        // Check for SSAI class
        guard let classAttr = attributes["CLASS"], classAttr.contains("ssai") || classAttr.contains("ad") else {
            return nil
        }
        
        guard let id = attributes["ID"] else {
            return nil
        }
        
        let duration = attributes["DURATION"].flatMap { TimeInterval($0) } ?? 0
        let startDate = attributes["START-DATE"] // ISO8601 date string
        
        // Determine cue type
        let cueType: AdCue.CueType
        if currentTime < 1 {
            cueType = .preroll
        } else if classAttr.contains("post") {
            cueType = .postroll
        } else {
            cueType = .midroll
        }
        
        let podId = attributes["X-POD-ID"] ?? id
        
        let cue = AdCue(
            id: id,
            startTime: currentTime,
            duration: duration,
            podId: podId,
            slotIndex: slotIndex,
            cueType: cueType,
            metadata: attributes
        )
        
        slotIndex += 1
        return cue
    }
    
    private func parseCueOut(_ line: String, at currentTime: TimeInterval, slotIndex: inout Int) -> AdCue? {
        var duration: TimeInterval = 0
        
        // Parse duration from CUE-OUT tag
        if line.contains(":") {
            if let durationStr = line.components(separatedBy: ":").last,
               let parsedDuration = TimeInterval(durationStr.trimmingCharacters(in: .whitespaces)) {
                duration = parsedDuration
            }
        }
        
        // Also check for DURATION attribute
        if line.contains("DURATION=") {
            let pattern = #"DURATION=([0-9.]+)"#
            if let regex = try? NSRegularExpression(pattern: pattern, options: []),
               let match = regex.firstMatch(in: line, range: NSRange(line.startIndex..., in: line)),
               let range = Range(match.range(at: 1), in: line),
               let parsedDuration = TimeInterval(line[range]) {
                duration = parsedDuration
            }
        }
        
        let cueType: AdCue.CueType = currentTime < 1 ? .preroll : .midroll
        let id = "cue-out-\(slotIndex)"
        
        let cue = AdCue(
            id: id,
            startTime: currentTime,
            duration: duration,
            podId: id,
            slotIndex: slotIndex,
            cueType: cueType,
            metadata: ["source": "EXT-X-CUE-OUT"]
        )
        
        slotIndex += 1
        return cue
    }
    
    private func parseDASHEvent(_ content: String, match: NSTextCheckingResult, slotIndex: inout Int) -> AdCue? {
        guard let range = Range(match.range, in: content) else { return nil }
        let eventXML = String(content[range])
        
        // Extract attributes from Event element
        let presentationTime = extractAttribute(from: eventXML, element: "Event", attribute: "presentationTime")
            .flatMap { TimeInterval($0) } ?? 0
        let duration = extractAttribute(from: eventXML, element: "Event", attribute: "duration")
            .flatMap { TimeInterval($0) } ?? 0
        let id = extractAttribute(from: eventXML, element: "Event", attribute: "id") ?? "event-\(slotIndex)"
        
        let cueType: AdCue.CueType = presentationTime < 1 ? .preroll : .midroll
        
        let cue = AdCue(
            id: id,
            startTime: presentationTime,
            duration: duration,
            podId: id,
            slotIndex: slotIndex,
            cueType: cueType,
            metadata: ["source": "DASH-EventStream"]
        )
        
        slotIndex += 1
        return cue
    }
    
    private func extractAttribute(from xml: String, element: String, attribute: String) -> String? {
        let pattern = "\(attribute)=\"([^\"]*)\""
        guard let regex = try? NSRegularExpression(pattern: pattern, options: []),
              let match = regex.firstMatch(in: xml, range: NSRange(xml.startIndex..., in: xml)),
              let range = Range(match.range(at: 1), in: xml) else {
            return nil
        }
        return String(xml[range])
    }
    
    private func parseISO8601Duration(_ duration: String) -> TimeInterval? {
        // Parse ISO 8601 duration (PT1H30M45S format)
        var totalSeconds: TimeInterval = 0
        var current = duration
        
        guard current.hasPrefix("PT") else { return nil }
        current = String(current.dropFirst(2))
        
        // Hours
        if let hIndex = current.firstIndex(of: "H") {
            let hours = current[..<hIndex]
            if let h = TimeInterval(hours) {
                totalSeconds += h * 3600
            }
            current = String(current[current.index(after: hIndex)...])
        }
        
        // Minutes
        if let mIndex = current.firstIndex(of: "M") {
            let minutes = current[..<mIndex]
            if let m = TimeInterval(minutes) {
                totalSeconds += m * 60
            }
            current = String(current[current.index(after: mIndex)...])
        }
        
        // Seconds
        if let sIndex = current.firstIndex(of: "S") {
            let seconds = current[..<sIndex]
            if let s = TimeInterval(seconds) {
                totalSeconds += s
            }
        }
        
        return totalSeconds > 0 ? totalSeconds : nil
    }
}
