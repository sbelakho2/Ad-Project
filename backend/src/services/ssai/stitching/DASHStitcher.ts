/**
 * DASH Stitcher
 *
 * Handles DASH-specific stitching operations including
 * multi-period management and EventStream handling.
 */

import {
  StitchingRequest,
  StitchingResult,
  StitchedSegment,
  AdSegment,
} from './types';

export interface DASHStitchingOptions {
  /** Minimum update period for live (ISO 8601 duration) */
  minimumUpdatePeriod: string;

  /** Availability start time for live */
  availabilityStartTime?: Date;

  /** Include EventStream for ad markers */
  includeEventStream: boolean;

  /** EventStream scheme URI */
  eventStreamScheme: string;

  /** Time shift buffer depth (ISO 8601 duration) */
  timeShiftBufferDepth?: string;

  /** Suggested presentation delay (ISO 8601 duration) */
  suggestedPresentationDelay?: string;
}

export interface DASHPeriod {
  /** Period ID */
  id: string;

  /** Period start time (ISO 8601 duration from MPD start) */
  start: string;

  /** Period duration (ISO 8601 duration, optional) */
  duration?: string;

  /** Adaptation sets in this period */
  adaptationSets: DASHAdaptationSet[];

  /** Is this an ad period */
  isAd: boolean;

  /** Creative ID if ad */
  creativeId?: string;

  /** Slot ID if ad */
  slotId?: string;
}

export interface DASHAdaptationSet {
  /** Adaptation set ID */
  id: string;

  /** Content type */
  contentType: 'video' | 'audio' | 'text';

  /** MIME type */
  mimeType: string;

  /** Codecs */
  codecs?: string;

  /** Representations */
  representations: DASHRepresentation[];

  /** Init segment template */
  initSegment?: string;

  /** Segment template */
  segmentTemplate?: DASHSegmentTemplate;
}

export interface DASHRepresentation {
  /** Representation ID */
  id: string;

  /** Bandwidth in bps */
  bandwidth: number;

  /** Width (video only) */
  width?: number;

  /** Height (video only) */
  height?: number;

  /** Segment list or override template */
  segmentList?: string[];
}

export interface DASHSegmentTemplate {
  /** Timescale */
  timescale: number;

  /** Duration */
  duration: number;

  /** Init template */
  initialization?: string;

  /** Media template */
  media?: string;

  /** Start number */
  startNumber?: number;

  /** Segment timeline */
  segmentTimeline?: Array<{
    t?: number;
    d: number;
    r?: number;
  }>;
}

/**
 * DASH Stitcher for seamless ad insertion into DASH manifests.
 */
export class DASHStitcher {
  private options: DASHStitchingOptions;

  constructor(options?: Partial<DASHStitchingOptions>) {
    this.options = {
      minimumUpdatePeriod: 'PT2S',
      includeEventStream: true,
      eventStreamScheme: 'urn:scte:scte35:2013:bin',
      ...options,
    };
  }

  /**
   * Stitch ads into a DASH manifest.
   */
  stitch(request: StitchingRequest): StitchingResult {
    const errors: string[] = [];
    const warnings: string[] = [];

    try {
      // Build period list
      const periods = this.buildPeriods(request);

      // Generate MPD
      const manifest = this.generateMPD(periods, request);

      // Build flat segment list for tracking
      const segments = this.flattenPeriodSegments(periods, request);

      return {
        success: true,
        manifest,
        segments,
        discontinuityCount: periods.length - 1,
        errors,
        warnings,
      };
    } catch (error) {
      errors.push(
        `DASH stitching failed: ${error instanceof Error ? error.message : String(error)}`
      );
      return {
        success: false,
        discontinuityCount: 0,
        errors,
        warnings,
      };
    }
  }

  /**
   * Build DASH periods from request.
   */
  private buildPeriods(request: StitchingRequest): DASHPeriod[] {
    const periods: DASHPeriod[] = [];
    let currentTime = 0;

    // Content period before break
    const beforeSegments = request.contentSegments.filter(
      (s) => s.position === 'before'
    );
    if (beforeSegments.length > 0) {
      const duration = beforeSegments.reduce((sum, s) => sum + s.duration, 0);
      periods.push(this.createContentPeriod('content-pre', currentTime, duration));
      currentTime += duration;
    }

    // Group ad segments by creative
    const adsByCreative = this.groupAdsByCreative(request.adSegments);

    // Ad periods (one per creative for clean period boundaries)
    for (const [creativeId, segments] of adsByCreative.entries()) {
      const duration = segments.reduce((sum, s) => sum + s.duration, 0);
      const slotId = segments[0].slotId;

      periods.push(
        this.createAdPeriod(
          `ad-${creativeId}`,
          currentTime,
          duration,
          creativeId,
          slotId,
          segments
        )
      );
      currentTime += duration;
    }

    // Content period after break
    const afterSegments = request.contentSegments.filter(
      (s) => s.position === 'after'
    );
    if (afterSegments.length > 0) {
      const duration = afterSegments.reduce((sum, s) => sum + s.duration, 0);
      periods.push(
        this.createContentPeriod('content-post', currentTime, duration)
      );
    }

    return periods;
  }

  /**
   * Create a content period.
   */
  private createContentPeriod(
    id: string,
    startSeconds: number,
    durationSeconds: number
  ): DASHPeriod {
    return {
      id,
      start: this.toISODuration(startSeconds),
      duration: this.toISODuration(durationSeconds),
      adaptationSets: this.createDefaultAdaptationSets(),
      isAd: false,
    };
  }

  /**
   * Create an ad period.
   */
  private createAdPeriod(
    id: string,
    startSeconds: number,
    durationSeconds: number,
    creativeId: string,
    slotId: string,
    _segments: AdSegment[]
  ): DASHPeriod {
    return {
      id,
      start: this.toISODuration(startSeconds),
      duration: this.toISODuration(durationSeconds),
      adaptationSets: this.createDefaultAdaptationSets(),
      isAd: true,
      creativeId,
      slotId,
    };
  }

  /**
   * Create default adaptation sets.
   */
  private createDefaultAdaptationSets(): DASHAdaptationSet[] {
    return [
      {
        id: '1',
        contentType: 'video',
        mimeType: 'video/mp4',
        codecs: 'avc1.64001f',
        representations: [
          { id: 'v1', bandwidth: 2000000, width: 1920, height: 1080 },
          { id: 'v2', bandwidth: 1000000, width: 1280, height: 720 },
        ],
      },
      {
        id: '2',
        contentType: 'audio',
        mimeType: 'audio/mp4',
        codecs: 'mp4a.40.2',
        representations: [{ id: 'a1', bandwidth: 128000 }],
      },
    ];
  }

  /**
   * Group ad segments by creative ID.
   */
  private groupAdsByCreative(
    adSegments: AdSegment[]
  ): Map<string, AdSegment[]> {
    const groups = new Map<string, AdSegment[]>();

    for (const segment of adSegments) {
      const existing = groups.get(segment.creativeId) || [];
      existing.push(segment);
      groups.set(segment.creativeId, existing);
    }

    return groups;
  }

  /**
   * Generate the DASH MPD content.
   */
  private generateMPD(
    periods: DASHPeriod[],
    request: StitchingRequest
  ): string {
    const lines: string[] = [];

    // XML declaration
    lines.push('<?xml version="1.0" encoding="UTF-8"?>');

    // MPD element
    const mpdAttrs = [
      'xmlns="urn:mpeg:dash:schema:mpd:2011"',
      'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"',
      'xsi:schemaLocation="urn:mpeg:dash:schema:mpd:2011 DASH-MPD.xsd"',
      `profiles="urn:mpeg:dash:profile:isoff-live:2011"`,
      `type="${request.isLive ? 'dynamic' : 'static'}"`,
    ];

    if (request.isLive) {
      mpdAttrs.push(`minimumUpdatePeriod="${this.options.minimumUpdatePeriod}"`);
      if (this.options.availabilityStartTime) {
        mpdAttrs.push(
          `availabilityStartTime="${this.options.availabilityStartTime.toISOString()}"`
        );
      }
      if (this.options.timeShiftBufferDepth) {
        mpdAttrs.push(
          `timeShiftBufferDepth="${this.options.timeShiftBufferDepth}"`
        );
      }
      if (this.options.suggestedPresentationDelay) {
        mpdAttrs.push(
          `suggestedPresentationDelay="${this.options.suggestedPresentationDelay}"`
        );
      }
    } else {
      // Calculate total duration for VOD
      const totalDuration = periods.reduce((sum, p) => {
        return sum + (p.duration ? this.parseISODuration(p.duration) : 0);
      }, 0);
      mpdAttrs.push(`mediaPresentationDuration="${this.toISODuration(totalDuration)}"`);
    }

    lines.push(`<MPD ${mpdAttrs.join(' ')}>`);

    // Add UTCTiming for live
    if (request.isLive) {
      lines.push(
        '  <UTCTiming schemeIdUri="urn:mpeg:dash:utc:http-xsdate:2014" value="https://time.akamai.com/?iso"/>'
      );
    }

    // Add periods
    for (const period of periods) {
      lines.push(this.generatePeriod(period));
    }

    lines.push('</MPD>');

    return lines.join('\n');
  }

  /**
   * Generate a Period element.
   */
  private generatePeriod(period: DASHPeriod): string {
    const lines: string[] = [];

    // Period element
    const periodAttrs = [`id="${period.id}"`, `start="${period.start}"`];
    if (period.duration) {
      periodAttrs.push(`duration="${period.duration}"`);
    }

    lines.push(`  <Period ${periodAttrs.join(' ')}>`);

    // Add EventStream for ad period
    if (period.isAd && this.options.includeEventStream) {
      lines.push(this.generateEventStream(period));
    }

    // Add adaptation sets
    for (const as of period.adaptationSets) {
      lines.push(this.generateAdaptationSet(as));
    }

    lines.push('  </Period>');

    return lines.join('\n');
  }

  /**
   * Generate EventStream for ad signaling.
   */
  private generateEventStream(period: DASHPeriod): string {
    const lines: string[] = [];

    lines.push(
      `    <EventStream schemeIdUri="${this.options.eventStreamScheme}" timescale="1000">`
    );
    lines.push(
      `      <Event presentationTime="0" duration="${this.parseISODuration(period.duration || 'PT0S') * 1000}" id="${period.creativeId || ''}">`
    );
    lines.push(`        <!-- Ad Creative: ${period.creativeId} -->`);
    lines.push('      </Event>');
    lines.push('    </EventStream>');

    return lines.join('\n');
  }

  /**
   * Generate AdaptationSet element.
   */
  private generateAdaptationSet(as: DASHAdaptationSet): string {
    const lines: string[] = [];

    const asAttrs = [
      `id="${as.id}"`,
      `contentType="${as.contentType}"`,
      `mimeType="${as.mimeType}"`,
    ];
    if (as.codecs) {
      asAttrs.push(`codecs="${as.codecs}"`);
    }

    lines.push(`    <AdaptationSet ${asAttrs.join(' ')}>`);

    // Add representations
    for (const rep of as.representations) {
      lines.push(this.generateRepresentation(rep, as.contentType));
    }

    lines.push('    </AdaptationSet>');

    return lines.join('\n');
  }

  /**
   * Generate Representation element.
   */
  private generateRepresentation(
    rep: DASHRepresentation,
    contentType: string
  ): string {
    const attrs = [`id="${rep.id}"`, `bandwidth="${rep.bandwidth}"`];

    if (contentType === 'video') {
      if (rep.width) attrs.push(`width="${rep.width}"`);
      if (rep.height) attrs.push(`height="${rep.height}"`);
    }

    return `      <Representation ${attrs.join(' ')}/>`;
  }

  /**
   * Flatten periods to segment list.
   */
  private flattenPeriodSegments(
    periods: DASHPeriod[],
    _request: StitchingRequest
  ): StitchedSegment[] {
    const segments: StitchedSegment[] = [];
    let sequence = 0;

    for (const period of periods) {
      const periodStart = this.parseISODuration(period.start);
      const periodDuration = period.duration
        ? this.parseISODuration(period.duration)
        : 0;

      // Add a representative segment for this period
      segments.push({
        uri: `period-${period.id}`,
        duration: periodDuration,
        mediaTime: periodStart,
        sequence: sequence++,
        isAd: period.isAd,
        creativeId: period.creativeId,
        slotId: period.slotId,
        discontinuity: true,
      });
    }

    return segments;
  }

  /**
   * Convert seconds to ISO 8601 duration.
   */
  private toISODuration(seconds: number): string {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;

    let duration = 'PT';
    if (hours > 0) duration += `${hours}H`;
    if (minutes > 0) duration += `${minutes}M`;
    if (secs > 0 || (hours === 0 && minutes === 0)) {
      duration += `${secs.toFixed(3).replace(/\.?0+$/, '')}S`;
    }

    return duration;
  }

  /**
   * Parse ISO 8601 duration to seconds.
   */
  private parseISODuration(duration: string): number {
    const match = duration.match(
      /PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+\.?\d*)S)?/i
    );
    if (!match) return 0;

    const hours = match[1] ? parseInt(match[1], 10) : 0;
    const minutes = match[2] ? parseInt(match[2], 10) : 0;
    const seconds = match[3] ? parseFloat(match[3]) : 0;

    return hours * 3600 + minutes * 60 + seconds;
  }

  /**
   * Validate DASH stitching.
   */
  validateStitching(periods: DASHPeriod[]): {
    valid: boolean;
    issues: string[];
  } {
    const issues: string[] = [];

    // Check for valid period structure
    for (let i = 0; i < periods.length; i++) {
      const period = periods[i];

      if (!period.id) {
        issues.push(`Period ${i} missing ID`);
      }

      if (period.adaptationSets.length === 0) {
        issues.push(`Period ${period.id} has no adaptation sets`);
      }

      // Check for video and audio
      const hasVideo = period.adaptationSets.some(
        (as) => as.contentType === 'video'
      );
      const hasAudio = period.adaptationSets.some(
        (as) => as.contentType === 'audio'
      );

      if (!hasVideo) {
        issues.push(`Period ${period.id} missing video adaptation set`);
      }
      if (!hasAudio) {
        issues.push(`Period ${period.id} missing audio adaptation set`);
      }
    }

    // Check period timing
    for (let i = 1; i < periods.length; i++) {
      const prev = periods[i - 1];
      const curr = periods[i];

      if (prev.duration) {
        const prevEnd =
          this.parseISODuration(prev.start) +
          this.parseISODuration(prev.duration);
        const currStart = this.parseISODuration(curr.start);

        if (Math.abs(prevEnd - currStart) > 0.001) {
          issues.push(
            `Gap or overlap between periods ${prev.id} and ${curr.id}`
          );
        }
      }
    }

    return {
      valid: issues.length === 0,
      issues,
    };
  }

  /**
   * Update options.
   */
  updateOptions(options: Partial<DASHStitchingOptions>): void {
    this.options = { ...this.options, ...options };
  }

  /**
   * Get current options.
   */
  getOptions(): DASHStitchingOptions {
    return { ...this.options };
  }
}

export default DASHStitcher;
