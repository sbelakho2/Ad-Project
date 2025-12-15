/**
 * HLS Cue Handler
 *
 * Parses and handles EXT-X-DATERANGE cues from HLS playlists.
 */

import {
  HLSDateRangeCue,
  AdBreakCue,
} from './types';
import { scte35Parser } from './SCTE35Parser';

/**
 * HLS Cue Handler
 */
export class HLSCueHandler {
  /**
   * Parse DATERANGE from HLS playlist line
   */
  parseDateRange(line: string): HLSDateRangeCue | null {
    if (!line.startsWith('#EXT-X-DATERANGE:')) {
      return null;
    }

    const attributeString = line.substring('#EXT-X-DATERANGE:'.length);
    const attributes = this.parseAttributes(attributeString);

    // ID and START-DATE are required
    const id = attributes.get('ID');
    const startDateStr = attributes.get('START-DATE');

    if (!id || !startDateStr) {
      return null;
    }

    const startDate = new Date(startDateStr);
    if (isNaN(startDate.getTime())) {
      return null;
    }

    const cue: HLSDateRangeCue = {
      id,
      startDate,
      customAttributes: {},
    };

    // Parse optional attributes
    const endDateStr = attributes.get('END-DATE');
    if (endDateStr) {
      cue.endDate = new Date(endDateStr);
    }

    const durationStr = attributes.get('DURATION');
    if (durationStr) {
      cue.duration = parseFloat(durationStr);
    }

    const plannedDurationStr = attributes.get('PLANNED-DURATION');
    if (plannedDurationStr) {
      cue.plannedDuration = parseFloat(plannedDurationStr);
    }

    const classAttr = attributes.get('CLASS');
    if (classAttr) {
      cue.class = classAttr;
    }

    // SCTE-35 attributes
    const scte35Cmd = attributes.get('SCTE35-CMD');
    if (scte35Cmd) {
      cue.scte35Cmd = scte35Cmd.replace(/^0x/i, '');
    }

    const scte35Out = attributes.get('SCTE35-OUT');
    if (scte35Out) {
      cue.scte35Out = scte35Out.replace(/^0x/i, '');
    }

    const scte35In = attributes.get('SCTE35-IN');
    if (scte35In) {
      cue.scte35In = scte35In.replace(/^0x/i, '');
    }

    const endOnNext = attributes.get('END-ON-NEXT');
    if (endOnNext === 'YES') {
      cue.endOnNext = true;
    }

    // Custom X- attributes
    for (const [key, value] of attributes.entries()) {
      if (key.startsWith('X-')) {
        // Try to parse as number
        const numValue = parseFloat(value);
        if (!isNaN(numValue) && value === numValue.toString()) {
          cue.customAttributes[key.substring(2)] = numValue;
        } else {
          cue.customAttributes[key.substring(2)] = value;
        }
      }
    }

    return cue;
  }

  /**
   * Parse all DATERANGE cues from playlist content
   */
  parseAllDateRanges(playlistContent: string): HLSDateRangeCue[] {
    const cues: HLSDateRangeCue[] = [];
    const lines = playlistContent.split('\n');

    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith('#EXT-X-DATERANGE:')) {
        const cue = this.parseDateRange(trimmed);
        if (cue) {
          cues.push(cue);
        }
      }
    }

    return cues;
  }

  /**
   * Convert DATERANGE cue to normalized ad break cue
   */
  toAdBreakCue(
    dateRange: HLSDateRangeCue,
    programDateTime?: Date
  ): AdBreakCue | null {
    // Determine cue type
    let cueType: 'start' | 'end' = 'start';

    // Check class for hints
    if (dateRange.class) {
      if (dateRange.class.toLowerCase().includes('return') ||
          dateRange.class.toLowerCase().includes('end')) {
        cueType = 'end';
      }
    }

    // Check SCTE-35 signals
    if (dateRange.scte35In) {
      cueType = 'end';
    }

    // Calculate media time
    let mediaTime = 0;
    if (programDateTime) {
      mediaTime = (dateRange.startDate.getTime() - programDateTime.getTime()) / 1000;
    }

    const cue: AdBreakCue = {
      cueId: `daterange_${dateRange.id}`,
      type: cueType,
      mediaTime,
      wallClockTime: dateRange.startDate,
      duration: dateRange.duration,
      plannedDuration: dateRange.plannedDuration,
      breakId: dateRange.id,
      source: 'daterange',
      original: dateRange,
    };

    // Try to parse embedded SCTE-35 for more detail
    if (dateRange.scte35Out) {
      try {
        const scte35 = scte35Parser.parseHex(dateRange.scte35Out);
        const scte35Cue = scte35Parser.toAdBreakCue(scte35, mediaTime);
        if (scte35Cue) {
          cue.segmentationType = scte35Cue.segmentationType;
          cue.availNum = scte35Cue.availNum;
          cue.availsExpected = scte35Cue.availsExpected;
          cue.autoReturn = scte35Cue.autoReturn;
          cue.rawScte35 = scte35.rawData;
        }
      } catch {
        // Failed to parse SCTE-35, continue without it
      }
    }

    return cue;
  }

  /**
   * Generate DATERANGE tag for manifest
   */
  generateDateRangeTag(cue: AdBreakCue, options: {
    includeScte35?: boolean;
    className?: string;
  } = {}): string {
    const attrs: string[] = [
      `ID="${cue.cueId}"`,
      `START-DATE="${cue.wallClockTime?.toISOString() || new Date().toISOString()}"`,
    ];

    if (options.className) {
      attrs.push(`CLASS="${options.className}"`);
    }

    if (cue.duration !== undefined) {
      attrs.push(`DURATION=${cue.duration.toFixed(3)}`);
    }

    if (cue.plannedDuration !== undefined) {
      attrs.push(`PLANNED-DURATION=${cue.plannedDuration.toFixed(3)}`);
    }

    if (options.includeScte35 && cue.rawScte35) {
      if (cue.type === 'start') {
        attrs.push(`SCTE35-OUT=0x${cue.rawScte35}`);
      } else {
        attrs.push(`SCTE35-IN=0x${cue.rawScte35}`);
      }
    }

    return `#EXT-X-DATERANGE:${attrs.join(',')}`;
  }

  /**
   * Parse attribute string into map
   */
  private parseAttributes(attrString: string): Map<string, string> {
    const attrs = new Map<string, string>();
    let current = '';
    let inQuotes = false;
    let key = '';

    for (let i = 0; i < attrString.length; i++) {
      const char = attrString[i];

      if (char === '"') {
        inQuotes = !inQuotes;
        continue;
      }

      if (char === '=' && !inQuotes && !key) {
        key = current.trim();
        current = '';
        continue;
      }

      if (char === ',' && !inQuotes) {
        if (key) {
          attrs.set(key, current.trim());
        }
        key = '';
        current = '';
        continue;
      }

      current += char;
    }

    // Don't forget the last attribute
    if (key) {
      attrs.set(key, current.trim());
    }

    return attrs;
  }

  /**
   * Find matching end cue for a start cue
   */
  findEndCue(
    startCue: HLSDateRangeCue,
    allCues: HLSDateRangeCue[]
  ): HLSDateRangeCue | null {
    // Look for END-ON-NEXT pattern
    if (startCue.endOnNext) {
      // Find next cue with same class
      for (const cue of allCues) {
        if (cue.startDate > startCue.startDate && cue.class === startCue.class) {
          return cue;
        }
      }
    }

    // Look for explicit end cue with matching ID pattern
    const baseId = startCue.id.replace(/-start$/i, '').replace(/-out$/i, '');

    for (const cue of allCues) {
      if (cue.id === `${baseId}-end` || cue.id === `${baseId}-in`) {
        return cue;
      }

      // Check for SCTE35-IN
      if (cue.scte35In && cue.startDate > startCue.startDate) {
        // Could be matching end cue
        // Would need to parse SCTE-35 to verify event ID matches
        return cue;
      }
    }

    return null;
  }

  /**
   * Calculate break duration from cue pair
   */
  calculateBreakDuration(
    startCue: HLSDateRangeCue,
    endCue?: HLSDateRangeCue | null
  ): number | null {
    // Use explicit DURATION if present
    if (startCue.duration !== undefined) {
      return startCue.duration;
    }

    // Use PLANNED-DURATION as fallback
    if (startCue.plannedDuration !== undefined) {
      return startCue.plannedDuration;
    }

    // Calculate from end date
    if (startCue.endDate) {
      return (startCue.endDate.getTime() - startCue.startDate.getTime()) / 1000;
    }

    // Calculate from end cue
    if (endCue) {
      return (endCue.startDate.getTime() - startCue.startDate.getTime()) / 1000;
    }

    return null;
  }
}

// Export singleton instance
export const hlsCueHandler = new HLSCueHandler();
