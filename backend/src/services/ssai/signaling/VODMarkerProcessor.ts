/**
 * VOD Marker Processor
 *
 * Detects and processes ad markers that were preserved during
 * live-to-VOD harvesting. These markers indicate where ad breaks
 * should be inserted in VOD content.
 */

import { AdBreakCue, CueType, SCTE35Message } from './types';

export interface VODMarker {
  /** Marker ID */
  id: string;

  /** Position in the VOD timeline (seconds) */
  position: number;

  /** Duration of the ad break (if known) */
  duration?: number;

  /** Type of marker */
  type: 'scte35' | 'daterange' | 'id3' | 'chapter' | 'custom';

  /** Original marker data */
  data?: string;

  /** SCTE-35 message if parsed */
  scte35?: SCTE35Message;

  /** Whether this marks the end of a break */
  isEndMarker: boolean;

  /** Associated break ID (if this is an end marker) */
  associatedBreakId?: string;
}

export interface VODMarkerResult {
  markers: VODMarker[];
  adBreaks: AdBreakCue[];
  errors: string[];
}

export interface ManifestMarkerContext {
  /** Manifest type */
  type: 'hls' | 'dash';

  /** Total duration of the VOD asset */
  totalDuration: number;

  /** Segment duration (for position estimation) */
  segmentDuration?: number;

  /** Asset ID */
  assetId?: string;
}

/**
 * VOD Marker Processor for detecting and normalizing ad cue points
 * in VOD content that was harvested from live streams.
 */
export class VODMarkerProcessor {
  /**
   * Process an HLS manifest for VOD markers.
   */
  processHLSManifest(
    manifestContent: string,
    context: ManifestMarkerContext
  ): VODMarkerResult {
    const markers: VODMarker[] = [];
    const errors: string[] = [];
    let currentPosition = 0;
    let segmentDuration = context.segmentDuration || 6;

    const lines = manifestContent.split('\n');

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();

      // Track position via EXTINF
      if (line.startsWith('#EXTINF:')) {
        const durationMatch = line.match(/#EXTINF:(\d+\.?\d*)/);
        if (durationMatch) {
          segmentDuration = parseFloat(durationMatch[1]);
        }
      }

      // Look for EXT-X-DATERANGE markers
      if (line.startsWith('#EXT-X-DATERANGE:')) {
        try {
          const marker = this.parseDateRangeMarker(line, currentPosition);
          if (marker) {
            markers.push(marker);
          }
        } catch (error) {
          errors.push(
            `Failed to parse DATERANGE at line ${i + 1}: ${error instanceof Error ? error.message : String(error)}`
          );
        }
      }

      // Look for EXT-X-CUE markers (some encoders use this)
      if (line.startsWith('#EXT-X-CUE:') || line.startsWith('#EXT-X-CUE-OUT:')) {
        try {
          const marker = this.parseCueMarker(line, currentPosition);
          if (marker) {
            markers.push(marker);
          }
        } catch (error) {
          errors.push(
            `Failed to parse CUE at line ${i + 1}: ${error instanceof Error ? error.message : String(error)}`
          );
        }
      }

      // Look for EXT-X-CUE-IN (end markers)
      if (line.startsWith('#EXT-X-CUE-IN')) {
        markers.push({
          id: `cue-in-${currentPosition}`,
          position: currentPosition,
          type: 'custom',
          isEndMarker: true,
        });
      }

      // Look for SCTE-35 in comments
      if (line.startsWith('#EXT-OATCLS-SCTE35:')) {
        try {
          const marker = this.parseSCTE35Comment(line, currentPosition);
          if (marker) {
            markers.push(marker);
          }
        } catch (error) {
          errors.push(
            `Failed to parse SCTE35 at line ${i + 1}: ${error instanceof Error ? error.message : String(error)}`
          );
        }
      }

      // Track chapter markers (some VOD systems use these)
      if (line.startsWith('#EXT-X-CHAPTER:')) {
        try {
          const marker = this.parseChapterMarker(line, currentPosition);
          if (marker) {
            markers.push(marker);
          }
        } catch (error) {
          errors.push(
            `Failed to parse chapter at line ${i + 1}: ${error instanceof Error ? error.message : String(error)}`
          );
        }
      }

      // Update position after segment
      if (!line.startsWith('#') && line.length > 0) {
        currentPosition += segmentDuration;
      }
    }

    // Convert markers to ad breaks
    const adBreaks = this.convertMarkersToAdBreaks(markers);

    return { markers, adBreaks, errors };
  }

  /**
   * Process a DASH manifest for VOD markers.
   */
  processDASHManifest(
    mpdContent: string,
    context: ManifestMarkerContext
  ): VODMarkerResult {
    const markers: VODMarker[] = [];
    const errors: string[] = [];

    // Extract EventStream elements
    const eventStreamRegex =
      /<EventStream[^>]*schemeIdUri="([^"]*)"[^>]*(?:timescale="(\d+)")?[^>]*>([\s\S]*?)<\/EventStream>/gi;

    let match: RegExpExecArray | null;
    while ((match = eventStreamRegex.exec(mpdContent)) !== null) {
      const [, schemeIdUri, timescaleStr, content] = match;
      const timescale = timescaleStr ? parseInt(timescaleStr, 10) : 1;

      try {
        const eventMarkers = this.parseEventStreamMarkers(
          content,
          schemeIdUri,
          timescale
        );
        markers.push(...eventMarkers);
      } catch (error) {
        errors.push(
          `Failed to parse EventStream: ${error instanceof Error ? error.message : String(error)}`
        );
      }
    }

    // Also look for Period boundaries which may indicate ad points
    const periodMarkers = this.extractPeriodBoundaryMarkers(mpdContent);
    markers.push(...periodMarkers);

    // Convert markers to ad breaks
    const adBreaks = this.convertMarkersToAdBreaks(markers);

    return { markers, adBreaks, errors };
  }

  /**
   * Parse EXT-X-DATERANGE marker.
   */
  private parseDateRangeMarker(line: string, position: number): VODMarker | null {
    const attrs = this.parseAttributes(line.substring('#EXT-X-DATERANGE:'.length));

    const id = attrs.ID || `daterange-${position}`;
    const duration = attrs.DURATION ? parseFloat(attrs.DURATION) : undefined;
    const plannedDuration = attrs['PLANNED-DURATION']
      ? parseFloat(attrs['PLANNED-DURATION'])
      : undefined;

    // Check if this is an ad-related marker
    const classAttr = attrs.CLASS || '';
    const isAdMarker =
      classAttr.includes('ad') ||
      classAttr.includes('scte35') ||
      attrs['SCTE35-OUT'] ||
      attrs['SCTE35-CMD'];

    if (!isAdMarker) {
      return null;
    }

    // Determine if this is an end marker
    const isEndMarker = classAttr.includes('-END') || !!attrs['SCTE35-IN'];

    return {
      id,
      position,
      duration: duration || plannedDuration,
      type: 'daterange',
      data: line,
      isEndMarker,
      associatedBreakId: isEndMarker
        ? attrs['X-ASSET-ID'] || attrs.ID?.replace('-end', '')
        : undefined,
    };
  }

  /**
   * Parse EXT-X-CUE marker.
   */
  private parseCueMarker(line: string, position: number): VODMarker | null {
    const isCueOut =
      line.startsWith('#EXT-X-CUE-OUT:') ||
      line.startsWith('#EXT-X-CUE:DURATION=');

    let duration: number | undefined;

    // Parse duration
    const durationMatch = line.match(/DURATION=(\d+\.?\d*)/);
    if (durationMatch) {
      duration = parseFloat(durationMatch[1]);
    } else {
      // Some formats just have the duration after the colon
      const simpleMatch = line.match(/:(\d+\.?\d*)$/);
      if (simpleMatch) {
        duration = parseFloat(simpleMatch[1]);
      }
    }

    return {
      id: `cue-${position}`,
      position,
      duration,
      type: 'custom',
      data: line,
      isEndMarker: !isCueOut,
    };
  }

  /**
   * Parse SCTE-35 comment marker.
   */
  private parseSCTE35Comment(line: string, position: number): VODMarker | null {
    const base64Data = line.substring('#EXT-OATCLS-SCTE35:'.length).trim();

    return {
      id: `scte35-${position}`,
      position,
      type: 'scte35',
      data: base64Data,
      isEndMarker: false, // Will be determined by SCTE-35 content
    };
  }

  /**
   * Parse chapter marker.
   */
  private parseChapterMarker(line: string, position: number): VODMarker | null {
    const attrs = this.parseAttributes(line.substring('#EXT-X-CHAPTER:'.length));

    // Chapters may indicate ad pod boundaries
    const title = attrs.TITLE || '';
    const isAdChapter =
      title.toLowerCase().includes('ad') ||
      title.toLowerCase().includes('commercial') ||
      title.toLowerCase().includes('break');

    if (!isAdChapter) {
      return null;
    }

    return {
      id: `chapter-${attrs.ID || position}`,
      position,
      duration: attrs.DURATION ? parseFloat(attrs.DURATION) : undefined,
      type: 'chapter',
      data: line,
      isEndMarker: false,
    };
  }

  /**
   * Parse EventStream markers from DASH.
   */
  private parseEventStreamMarkers(
    content: string,
    schemeIdUri: string,
    timescale: number
  ): VODMarker[] {
    const markers: VODMarker[] = [];

    const eventRegex =
      /<Event[^>]*(?:id="([^"]*)")?[^>]*(?:presentationTime="(\d+)")?[^>]*(?:duration="(\d+)")?[^>]*(?:>([^<]*)<\/Event>|\/>)/gi;

    let match: RegExpExecArray | null;
    while ((match = eventRegex.exec(content)) !== null) {
      const [, id, presentationTimeStr, durationStr, messageData] = match;

      const presentationTime = presentationTimeStr
        ? parseInt(presentationTimeStr, 10)
        : 0;
      const duration = durationStr ? parseInt(durationStr, 10) : undefined;

      const position = presentationTime / timescale;
      const durationSeconds = duration ? duration / timescale : undefined;

      // Determine type based on scheme
      let type: VODMarker['type'] = 'custom';
      if (schemeIdUri.includes('scte35')) {
        type = 'scte35';
      } else if (schemeIdUri.includes('daterange')) {
        type = 'daterange';
      }

      markers.push({
        id: id || `event-${position}`,
        position,
        duration: durationSeconds,
        type,
        data: messageData?.trim(),
        isEndMarker: false,
      });
    }

    return markers;
  }

  /**
   * Extract ad markers from Period boundaries.
   */
  private extractPeriodBoundaryMarkers(mpdContent: string): VODMarker[] {
    const markers: VODMarker[] = [];

    // Look for Period elements with ad-related IDs
    const periodRegex =
      /<Period[^>]*id="([^"]*)"[^>]*(?:start="([^"]*)")?[^>]*(?:duration="([^"]*)")?/gi;

    let match: RegExpExecArray | null;
    while ((match = periodRegex.exec(mpdContent)) !== null) {
      const [, id, startStr, durationStr] = match;

      // Check if this is an ad period
      const isAdPeriod =
        id.toLowerCase().includes('ad') ||
        id.toLowerCase().includes('break') ||
        id.toLowerCase().includes('pod');

      if (!isAdPeriod) {
        continue;
      }

      const position = this.parseISODuration(startStr) || 0;
      const duration = this.parseISODuration(durationStr);

      markers.push({
        id: `period-${id}`,
        position,
        duration,
        type: 'custom',
        isEndMarker: false,
      });
    }

    return markers;
  }

  /**
   * Convert markers to normalized ad breaks.
   */
  private convertMarkersToAdBreaks(markers: VODMarker[]): AdBreakCue[] {
    const adBreaks: AdBreakCue[] = [];
    const startMarkers = markers.filter((m) => !m.isEndMarker);
    const endMarkers = markers.filter((m) => m.isEndMarker);

    for (const marker of startMarkers) {
      // Find matching end marker
      let duration = marker.duration;
      if (!duration) {
        const endMarker = endMarkers.find(
          (em) =>
            em.associatedBreakId === marker.id ||
            (em.position > marker.position && em.position - marker.position < 300) // Max 5 min break
        );
        if (endMarker) {
          duration = endMarker.position - marker.position;
        }
      }

      const cue: AdBreakCue = {
        cueId: marker.id,
        type: 'start',
        mediaTime: marker.position,
        duration,
        plannedDuration: duration,
        source: marker.type === 'scte35' ? 'scte35' : 
                marker.type === 'daterange' ? 'daterange' : 'schedule',
      };

      adBreaks.push(cue);
    }

    return adBreaks;
  }

  /**
   * Parse HLS-style attributes.
   */
  private parseAttributes(attrString: string): Record<string, string> {
    const attrs: Record<string, string> = {};
    const regex = /([A-Z0-9-]+)=(?:"([^"]*)"|([^,]*))/gi;

    let match: RegExpExecArray | null;
    while ((match = regex.exec(attrString)) !== null) {
      const [, key, quotedValue, unquotedValue] = match;
      attrs[key] = quotedValue !== undefined ? quotedValue : unquotedValue;
    }

    return attrs;
  }

  /**
   * Parse ISO 8601 duration (PT1H30M45S format).
   */
  private parseISODuration(durationStr?: string): number | undefined {
    if (!durationStr) {
      return undefined;
    }

    const match = durationStr.match(
      /PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+\.?\d*)S)?/i
    );
    if (!match) {
      return undefined;
    }

    const hours = match[1] ? parseInt(match[1], 10) : 0;
    const minutes = match[2] ? parseInt(match[2], 10) : 0;
    const seconds = match[3] ? parseFloat(match[3]) : 0;

    return hours * 3600 + minutes * 60 + seconds;
  }

  /**
   * Merge adjacent markers that represent the same break.
   */
  mergeAdjacentBreaks(
    adBreaks: AdBreakCue[],
    toleranceSeconds = 1
  ): AdBreakCue[] {
    if (adBreaks.length <= 1) {
      return adBreaks;
    }

    const sorted = [...adBreaks].sort((a, b) => a.mediaTime - b.mediaTime);
    const merged: AdBreakCue[] = [];

    let current = sorted[0];

    for (let i = 1; i < sorted.length; i++) {
      const next = sorted[i];
      const currentEnd = current.mediaTime + (current.duration || 0);

      // Check if breaks should be merged
      if (next.mediaTime - currentEnd <= toleranceSeconds) {
        // Merge into current
        const newDuration =
          next.mediaTime +
          (next.duration || 0) -
          current.mediaTime;
        current = {
          ...current,
          duration: newDuration,
          plannedDuration: newDuration,
        };
      } else {
        merged.push(current);
        current = next;
      }
    }

    merged.push(current);
    return merged;
  }
}

export default VODMarkerProcessor;
