/**
 * HLS Manifest Generator
 *
 * Generates RFC 8216 compliant HLS playlists for SSAI.
 * Supports both TS and CMAF/fMP4 segments.
 */

import {
  ManifestContext,
  SegmentInfo,
  RenditionInfo,
  AudioRenditionInfo,
  DateRangeCue,
  HLSManifestOptions,
  ManifestValidationResult,
  ManifestValidationError,
  ManifestValidationWarning,
} from './types';

/**
 * Default HLS options
 */
const DEFAULT_HLS_OPTIONS: HLSManifestOptions = {
  includeProgramDateTime: true,
  includeDateRange: true,
  liveWindowSize: 6,
};

/**
 * HLS Manifest Generator
 */
export class HLSManifestGenerator {
  private options: HLSManifestOptions;

  constructor(options: Partial<HLSManifestOptions> = {}) {
    this.options = { ...DEFAULT_HLS_OPTIONS, ...options };
  }

  /**
   * Generate master playlist
   */
  generateMasterPlaylist(
    context: ManifestContext,
    renditions: RenditionInfo[],
    audioRenditions?: AudioRenditionInfo[]
  ): string {
    const lines: string[] = ['#EXTM3U', '#EXT-X-VERSION:6'];

    // Add audio renditions if present
    if (audioRenditions && audioRenditions.length > 0) {
      for (const audio of audioRenditions) {
        const attrs = [
          `TYPE=AUDIO`,
          `GROUP-ID="${audio.groupId}"`,
          `LANGUAGE="${audio.language}"`,
          `NAME="${audio.name}"`,
          `DEFAULT=${audio.default ? 'YES' : 'NO'}`,
          `AUTOSELECT=${audio.autoselect ? 'YES' : 'NO'}`,
        ];

        if (audio.channels) {
          attrs.push(`CHANNELS="${audio.channels}"`);
        }

        if (audio.uri) {
          attrs.push(`URI="${this.appendToken(audio.uri, context.token)}"`);
        }

        lines.push(`#EXT-X-MEDIA:${attrs.join(',')}`);
      }
    }

    // Add stream variants
    for (const rendition of renditions) {
      const attrs = [
        `BANDWIDTH=${rendition.bandwidth}`,
        `CODECS="${rendition.codecs}"`,
      ];

      if (rendition.width && rendition.height) {
        attrs.push(`RESOLUTION=${rendition.width}x${rendition.height}`);
      }

      if (rendition.frameRate) {
        attrs.push(`FRAME-RATE=${rendition.frameRate.toFixed(3)}`);
      }

      if (rendition.audioGroup) {
        attrs.push(`AUDIO="${rendition.audioGroup}"`);
      }

      if (rendition.subtitleGroup) {
        attrs.push(`SUBTITLES="${rendition.subtitleGroup}"`);
      }

      if (rendition.closedCaptionsGroup) {
        if (rendition.closedCaptionsGroup === 'NONE') {
          attrs.push(`CLOSED-CAPTIONS=NONE`);
        } else {
          attrs.push(`CLOSED-CAPTIONS="${rendition.closedCaptionsGroup}"`);
        }
      }

      lines.push(`#EXT-X-STREAM-INF:${attrs.join(',')}`);
      lines.push(this.appendToken(rendition.uri, context.token));
    }

    return lines.join('\n') + '\n';
  }

  /**
   * Generate media playlist
   */
  generateMediaPlaylist(
    context: ManifestContext,
    segments: SegmentInfo[],
    cues?: DateRangeCue[]
  ): string {
    const lines: string[] = ['#EXTM3U', '#EXT-X-VERSION:6'];

    // Target duration (must be >= max segment duration)
    const maxDuration = Math.max(...segments.map((s) => s.duration), context.targetDuration);
    lines.push(`#EXT-X-TARGETDURATION:${Math.ceil(maxDuration)}`);

    // Media sequence - MUST be monotonic
    lines.push(`#EXT-X-MEDIA-SEQUENCE:${context.mediaSequence}`);

    // Discontinuity sequence - MUST be monotonic and consistent
    if (context.discontinuitySequence > 0) {
      lines.push(`#EXT-X-DISCONTINUITY-SEQUENCE:${context.discontinuitySequence}`);
    }

    // For VOD/event, add playlist type
    // NOTE: Live playlists must NOT include EXT-X-PLAYLIST-TYPE
    if (context.mode === 'vod') {
      lines.push('#EXT-X-PLAYLIST-TYPE:VOD');
    } else if (context.mode === 'event') {
      lines.push('#EXT-X-PLAYLIST-TYPE:EVENT');
    }
    // For 'live' mode, intentionally omit EXT-X-PLAYLIST-TYPE

    // Add DATERANGE cues if enabled
    if (this.options.includeDateRange && cues && cues.length > 0) {
      for (const cue of cues) {
        lines.push(this.formatDateRange(cue));
      }
    }

    // Track discontinuities for consistency
    let discontinuityCount = 0;

    // Add segments
    for (let i = 0; i < segments.length; i++) {
      const segment = segments[i];

      // Add discontinuity tag before segment if needed
      if (segment.discontinuity) {
        lines.push('#EXT-X-DISCONTINUITY');
        discontinuityCount++;
      }

      // Add program date time if enabled and present
      if (this.options.includeProgramDateTime && segment.programDateTime) {
        lines.push(`#EXT-X-PROGRAM-DATE-TIME:${segment.programDateTime}`);
      }

      // Add byte range for fMP4
      if (segment.byteRange) {
        lines.push(
          `#EXT-X-BYTERANGE:${segment.byteRange.length}@${segment.byteRange.offset}`
        );
      }

      // EXTINF - duration MUST be non-zero
      const duration = Math.max(segment.duration, 0.001);
      lines.push(`#EXTINF:${duration.toFixed(6)},`);

      // Segment URI with token
      lines.push(this.appendToken(segment.uri, context.token));
    }

    // Add ENDLIST for VOD (required) and event when complete
    if (context.mode === 'vod') {
      lines.push('#EXT-X-ENDLIST');
    }

    return lines.join('\n') + '\n';
  }

  /**
   * Format a DATERANGE cue
   */
  private formatDateRange(cue: DateRangeCue): string {
    const attrs: string[] = [`ID="${cue.id}"`, `START-DATE="${cue.startDate}"`];

    if (cue.class) {
      attrs.push(`CLASS="${cue.class}"`);
    }

    if (cue.endDate) {
      attrs.push(`END-DATE="${cue.endDate}"`);
    }

    if (cue.duration !== undefined) {
      attrs.push(`DURATION=${cue.duration.toFixed(3)}`);
    }

    if (cue.plannedDuration !== undefined) {
      attrs.push(`PLANNED-DURATION=${cue.plannedDuration.toFixed(3)}`);
    }

    if (cue.scte35Cmd) {
      attrs.push(`SCTE35-CMD=0x${cue.scte35Cmd}`);
    }

    if (cue.scte35Out) {
      attrs.push(`SCTE35-OUT=0x${cue.scte35Out}`);
    }

    if (cue.scte35In) {
      attrs.push(`SCTE35-IN=0x${cue.scte35In}`);
    }

    // Add custom attributes
    if (cue.customAttributes) {
      for (const [key, value] of Object.entries(cue.customAttributes)) {
        if (typeof value === 'string') {
          attrs.push(`X-${key}="${value}"`);
        } else {
          attrs.push(`X-${key}=${value}`);
        }
      }
    }

    return `#EXT-X-DATERANGE:${attrs.join(',')}`;
  }

  /**
   * Append token to URL
   */
  private appendToken(url: string, token: string): string {
    const separator = url.includes('?') ? '&' : '?';
    return `${url}${separator}token=${token}`;
  }

  /**
   * Validate a media playlist
   */
  validateMediaPlaylist(
    content: string,
    context: ManifestContext
  ): ManifestValidationResult {
    const errors: ManifestValidationError[] = [];
    const warnings: ManifestValidationWarning[] = [];
    const lines = content.split('\n');

    let hasExtm3u = false;
    let hasTargetDuration = false;
    let hasMediaSequence = false;
    let hasEndList = false;
    let hasPlaylistType = false;
    let previousMediaSequence = -1;
    let discontinuityCount = 0;
    let discontinuitySequence = 0;
    let lineNumber = 0;

    for (const line of lines) {
      lineNumber++;
      const trimmed = line.trim();

      if (trimmed === '#EXTM3U') {
        hasExtm3u = true;
      }

      if (trimmed.startsWith('#EXT-X-TARGETDURATION:')) {
        hasTargetDuration = true;
        const value = parseInt(trimmed.split(':')[1], 10);
        if (value <= 0) {
          errors.push({
            code: 'INVALID_TARGET_DURATION',
            message: 'EXT-X-TARGETDURATION must be positive',
            line: lineNumber,
          });
        }
      }

      if (trimmed.startsWith('#EXT-X-MEDIA-SEQUENCE:')) {
        hasMediaSequence = true;
        const value = parseInt(trimmed.split(':')[1], 10);
        if (previousMediaSequence >= 0 && value < previousMediaSequence) {
          errors.push({
            code: 'MEDIA_SEQUENCE_DECREASED',
            message: 'EXT-X-MEDIA-SEQUENCE must never decrease',
            line: lineNumber,
          });
        }
        previousMediaSequence = value;
      }

      if (trimmed.startsWith('#EXT-X-DISCONTINUITY-SEQUENCE:')) {
        discontinuitySequence = parseInt(trimmed.split(':')[1], 10);
      }

      if (trimmed === '#EXT-X-DISCONTINUITY') {
        discontinuityCount++;
      }

      if (trimmed.startsWith('#EXT-X-PLAYLIST-TYPE:')) {
        hasPlaylistType = true;
        if (context.mode === 'live') {
          errors.push({
            code: 'LIVE_HAS_PLAYLIST_TYPE',
            message: 'Live playlists must NOT include EXT-X-PLAYLIST-TYPE',
            line: lineNumber,
          });
        }
      }

      if (trimmed === '#EXT-X-ENDLIST') {
        hasEndList = true;
      }

      if (trimmed.startsWith('#EXTINF:')) {
        const durationStr = trimmed.split(':')[1].replace(',', '');
        const duration = parseFloat(durationStr);
        if (duration <= 0) {
          errors.push({
            code: 'ZERO_EXTINF',
            message: 'EXTINF duration must be non-zero',
            line: lineNumber,
          });
        }
      }
    }

    // Check required tags
    if (!hasExtm3u) {
      errors.push({
        code: 'MISSING_EXTM3U',
        message: 'Playlist must start with #EXTM3U',
        line: 1,
      });
    }

    if (!hasTargetDuration) {
      errors.push({
        code: 'MISSING_TARGET_DURATION',
        message: 'Playlist must have EXT-X-TARGETDURATION',
      });
    }

    // VOD must have ENDLIST
    if (context.mode === 'vod' && !hasEndList) {
      errors.push({
        code: 'VOD_MISSING_ENDLIST',
        message: 'VOD playlists must end with EXT-X-ENDLIST',
      });
    }

    // Live must not have PLAYLIST-TYPE (already checked above)

    return {
      valid: errors.length === 0,
      errors,
      warnings,
    };
  }

  /**
   * Calculate sliding window for live playlist
   *
   * Ensures media sequence increments correctly when segments slide out
   */
  applyLiveSlidingWindow(
    segments: SegmentInfo[],
    currentMediaSequence: number,
    currentDiscontinuitySequence: number
  ): {
    segments: SegmentInfo[];
    mediaSequence: number;
    discontinuitySequence: number;
  } {
    const windowSize = this.options.liveWindowSize;

    if (segments.length <= windowSize) {
      return {
        segments,
        mediaSequence: currentMediaSequence,
        discontinuitySequence: currentDiscontinuitySequence,
      };
    }

    // Calculate how many segments to remove
    const removeCount = segments.length - windowSize;

    // Count discontinuities in removed segments
    let discontinuitiesRemoved = 0;
    for (let i = 0; i < removeCount; i++) {
      if (segments[i].discontinuity) {
        discontinuitiesRemoved++;
      }
    }

    // Slide the window
    const newSegments = segments.slice(removeCount);

    // Increment media sequence by number of removed segments
    const newMediaSequence = currentMediaSequence + removeCount;

    // Increment discontinuity sequence by number of removed discontinuities
    const newDiscontinuitySequence =
      currentDiscontinuitySequence + discontinuitiesRemoved;

    return {
      segments: newSegments,
      mediaSequence: newMediaSequence,
      discontinuitySequence: newDiscontinuitySequence,
    };
  }
}

// Export singleton instance
export const hlsManifestGenerator = new HLSManifestGenerator();
