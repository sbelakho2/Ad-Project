/**
 * DASH Manifest Generator
 *
 * Generates DASH-IF compliant MPD manifests for SSAI.
 * Supports single-period and multi-period outputs.
 */

import {
  ManifestContext,
  DASHPeriod,
  DASHAdaptationSet,
  DASHRepresentation,
  DASHEventStream,
  DASHSegmentTemplate,
  DASHManifestOptions,
  ManifestValidationResult,
  ManifestValidationError,
  ManifestValidationWarning,
} from './types';

import { recordPerformanceMetric } from '../../performanceMetricsRecorder';

/**
 * Default DASH options
 */
const DEFAULT_DASH_OPTIONS: DASHManifestOptions = {
  minBufferTime: 'PT2S',
  minUpdatePeriod: 'PT2S',
  suggestedPresentationDelay: 'PT10S',
  timeShiftBufferDepth: 'PT5M',
  utcTiming: {
    schemeIdUri: 'urn:mpeg:dash:utc:http-xsdate:2014',
    value: 'https://time.akamai.com/?iso',
  },
};

/**
 * DASH Manifest Generator
 */
export class DASHManifestGenerator {
  private options: DASHManifestOptions;

  constructor(options: Partial<DASHManifestOptions> = {}) {
    this.options = { ...DEFAULT_DASH_OPTIONS, ...options };
  }

  /**
   * Generate MPD manifest
   */
  generateMPD(
    context: ManifestContext,
    periods: DASHPeriod[],
    totalDuration?: string
  ): string {
    const lines: string[] = [];

    // XML declaration
    lines.push('<?xml version="1.0" encoding="UTF-8"?>');

    // MPD element
    const mpdAttrs = this.buildMPDAttributes(context, totalDuration);
    lines.push(`<MPD ${mpdAttrs.join(' ')}>`);

    // Program Information
    lines.push('  <ProgramInformation>');
    lines.push('    <Title>Apex Mediation SSAI Stream</Title>');
    lines.push('  </ProgramInformation>');

    // UTC Timing for live
    if (context.mode === 'live' && this.options.utcTiming) {
      lines.push(
        `  <UTCTiming schemeIdUri="${this.options.utcTiming.schemeIdUri}" value="${this.options.utcTiming.value}"/>`
      );
    }

    // Base URL
    lines.push(`  <BaseURL>${context.segmentBaseUrl}</BaseURL>`);

    // Periods
    for (const period of periods) {
      lines.push(this.generatePeriod(period, context.token));
    }

    lines.push('</MPD>');

    return lines.join('\n');
  }

  /**
   * Build MPD element attributes
   */
  private buildMPDAttributes(context: ManifestContext, totalDuration?: string): string[] {
    const attrs: string[] = [
      'xmlns="urn:mpeg:dash:schema:mpd:2011"',
      'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"',
      'xsi:schemaLocation="urn:mpeg:dash:schema:mpd:2011 DASH-MPD.xsd"',
      'profiles="urn:mpeg:dash:profile:isoff-live:2011,urn:com:dashif:dash264"',
      `minBufferTime="${this.options.minBufferTime}"`,
    ];

    if (context.mode === 'live') {
      attrs.push('type="dynamic"');
      attrs.push(`publishTime="${new Date().toISOString()}"`);
      attrs.push(`availabilityStartTime="${this.getAvailabilityStartTime()}"`);

      if (this.options.minUpdatePeriod) {
        attrs.push(`minimumUpdatePeriod="${this.options.minUpdatePeriod}"`);
      }

      if (this.options.suggestedPresentationDelay) {
        attrs.push(
          `suggestedPresentationDelay="${this.options.suggestedPresentationDelay}"`
        );
      }

      if (this.options.timeShiftBufferDepth) {
        attrs.push(`timeShiftBufferDepth="${this.options.timeShiftBufferDepth}"`);
      }
    } else {
      attrs.push('type="static"');

      if (totalDuration) {
        attrs.push(`mediaPresentationDuration="${totalDuration}"`);
      }
    }

    return attrs;
  }

  /**
   * Get availability start time for live streams
   */
  private getAvailabilityStartTime(): string {
    // In production, this would be the actual stream start time
    const startTime = new Date();
    startTime.setHours(startTime.getHours() - 1); // 1 hour ago
    return startTime.toISOString();
  }

  /**
   * Generate Period element
   */
  private generatePeriod(period: DASHPeriod, token: string): string {
    const lines: string[] = [];

    const periodAttrs = [`id="${period.id}"`, `start="${period.start}"`];

    if (period.duration) {
      periodAttrs.push(`duration="${period.duration}"`);
    }

    lines.push(`  <Period ${periodAttrs.join(' ')}>`);

    // Event streams (for SCTE-35)
    if (period.eventStreams) {
      for (const eventStream of period.eventStreams) {
        lines.push(this.generateEventStream(eventStream));
      }
    }

    // Adaptation sets
    for (const adaptationSet of period.adaptationSets) {
      lines.push(this.generateAdaptationSet(adaptationSet, token));
    }

    lines.push('  </Period>');

    return lines.join('\n');
  }

  /**
   * Generate EventStream element
   */
  private generateEventStream(eventStream: DASHEventStream): string {
    const lines: string[] = [];

    const attrs = [`schemeIdUri="${eventStream.schemeIdUri}"`];

    if (eventStream.value) {
      attrs.push(`value="${eventStream.value}"`);
    }

    if (eventStream.timescale) {
      attrs.push(`timescale="${eventStream.timescale}"`);
    }

    lines.push(`    <EventStream ${attrs.join(' ')}>`);

    for (const event of eventStream.events) {
      const eventAttrs: string[] = [];

      if (event.presentationTime !== undefined) {
        eventAttrs.push(`presentationTime="${event.presentationTime}"`);
      }

      if (event.duration !== undefined) {
        eventAttrs.push(`duration="${event.duration}"`);
      }

      if (event.id !== undefined) {
        eventAttrs.push(`id="${event.id}"`);
      }

      if (event.messageData) {
        lines.push(`      <Event ${eventAttrs.join(' ')}>${event.messageData}</Event>`);
      } else {
        lines.push(`      <Event ${eventAttrs.join(' ')}/>`);
      }
    }

    lines.push('    </EventStream>');

    return lines.join('\n');
  }

  /**
   * Generate AdaptationSet element
   */
  private generateAdaptationSet(adaptationSet: DASHAdaptationSet, token: string): string {
    const lines: string[] = [];

    const attrs = [
      `id="${adaptationSet.id}"`,
      `contentType="${adaptationSet.contentType}"`,
      `mimeType="${adaptationSet.mimeType}"`,
      `segmentAlignment="${adaptationSet.segmentAlignment}"`,
    ];

    if (adaptationSet.codecs) {
      attrs.push(`codecs="${adaptationSet.codecs}"`);
    }

    if (adaptationSet.lang) {
      attrs.push(`lang="${adaptationSet.lang}"`);
    }

    lines.push(`    <AdaptationSet ${attrs.join(' ')}>`);

    // Representations
    for (const representation of adaptationSet.representations) {
      lines.push(this.generateRepresentation(representation, token));
    }

    lines.push('    </AdaptationSet>');

    return lines.join('\n');
  }

  /**
   * Generate Representation element
   */
  private generateRepresentation(representation: DASHRepresentation, token: string): string {
    const lines: string[] = [];

    const attrs = [
      `id="${representation.id}"`,
      `bandwidth="${representation.bandwidth}"`,
    ];

    if (representation.width) {
      attrs.push(`width="${representation.width}"`);
    }

    if (representation.height) {
      attrs.push(`height="${representation.height}"`);
    }

    if (representation.codecs) {
      attrs.push(`codecs="${representation.codecs}"`);
    }

    if (representation.frameRate) {
      attrs.push(`frameRate="${representation.frameRate}"`);
    }

    lines.push(`      <Representation ${attrs.join(' ')}>`);

    // Segment template
    if (representation.segmentTemplate) {
      lines.push(
        this.generateSegmentTemplate(representation.segmentTemplate, token)
      );
    }

    lines.push('      </Representation>');

    return lines.join('\n');
  }

  /**
   * Generate SegmentTemplate element
   */
  private generateSegmentTemplate(template: DASHSegmentTemplate, token: string): string {
    const lines: string[] = [];

    const attrs = [`timescale="${template.timescale}"`];

    if (template.presentationTimeOffset !== undefined) {
      attrs.push(`presentationTimeOffset="${template.presentationTimeOffset}"`);
    }

    // Append token to media template
    const mediaUrl = this.appendToken(template.media, token);
    attrs.push(`media="${mediaUrl}"`);

    if (template.initialization) {
      const initUrl = this.appendToken(template.initialization, token);
      attrs.push(`initialization="${initUrl}"`);
    }

    if (template.startNumber !== undefined) {
      attrs.push(`startNumber="${template.startNumber}"`);
    }

    if (template.duration !== undefined) {
      attrs.push(`duration="${template.duration}"`);
    }

    if (template.segmentTimeline) {
      lines.push(`        <SegmentTemplate ${attrs.join(' ')}>`);
      lines.push('          <SegmentTimeline>');

      for (const seg of template.segmentTimeline.segments) {
        const segAttrs: string[] = [];

        if (seg.t !== undefined) {
          segAttrs.push(`t="${seg.t}"`);
        }

        segAttrs.push(`d="${seg.d}"`);

        if (seg.r !== undefined && seg.r > 0) {
          segAttrs.push(`r="${seg.r}"`);
        }

        lines.push(`            <S ${segAttrs.join(' ')}/>`);
      }

      lines.push('          </SegmentTimeline>');
      lines.push('        </SegmentTemplate>');
    } else {
      lines.push(`        <SegmentTemplate ${attrs.join(' ')}/>`);
    }

    return lines.join('\n');
  }

  /**
   * Append token to URL template
   */
  private appendToken(url: string, token: string): string {
    // Handle URL templates with $...$ placeholders
    const separator = url.includes('?') ? '&amp;' : '?';
    return `${url}${separator}token=${token}`;
  }

  /**
   * Convert single-period to multi-period for SSAI
   *
   * This is needed when inserting ads into a single-period origin stream.
   */
  convertToMultiPeriod(
    singlePeriod: DASHPeriod,
    adPeriods: DASHPeriod[],
    insertionPoints: number[] // Times in seconds where to insert ad periods
  ): DASHPeriod[] {
    if (adPeriods.length === 0 || insertionPoints.length === 0) {
      return [singlePeriod];
    }

    const result: DASHPeriod[] = [];
    let contentOffset = 0;
    let periodIndex = 0;

    // Sort insertion points
    const sortedPoints = [...insertionPoints].sort((a, b) => a - b);

    for (let i = 0; i < sortedPoints.length; i++) {
      const insertTime = sortedPoints[i];
      const adPeriod = adPeriods[i];

      if (!adPeriod) continue;

      // Add content period before this insertion point
      if (insertTime > contentOffset) {
        const contentDuration = insertTime - contentOffset;

        result.push({
          ...singlePeriod,
          id: `content-${periodIndex}`,
          start: this.secondsToDuration(contentOffset),
          duration: this.secondsToDuration(contentDuration),
          isAd: false,
        });

        periodIndex++;
      }

      // Add ad period
      result.push({
        ...adPeriod,
        id: `ad-${periodIndex}`,
        start: this.secondsToDuration(insertTime),
        isAd: true,
      });

      // Update content offset
      const adDurationSeconds = this.durationToSeconds(adPeriod.duration || 'PT0S');
      contentOffset = insertTime + adDurationSeconds;
      periodIndex++;
    }

    // Add remaining content
    // In a real implementation, you'd calculate the remaining duration
    result.push({
      ...singlePeriod,
      id: `content-${periodIndex}`,
      start: this.secondsToDuration(contentOffset),
      isAd: false,
    });

    return result;
  }

  /**
   * Convert seconds to ISO 8601 duration
   */
  private secondsToDuration(seconds: number): string {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;

    let result = 'PT';

    if (hours > 0) {
      result += `${hours}H`;
    }

    if (minutes > 0) {
      result += `${minutes}M`;
    }

    if (secs > 0 || result === 'PT') {
      result += `${secs.toFixed(3)}S`;
    }

    return result;
  }

  /**
   * Convert ISO 8601 duration to seconds
   */
  private durationToSeconds(duration: string): number {
    const match = duration.match(/PT(?:(\d+)H)?(?:(\d+)M)?(?:([\d.]+)S)?/);

    if (!match) return 0;

    const hours = parseInt(match[1] || '0', 10);
    const minutes = parseInt(match[2] || '0', 10);
    const seconds = parseFloat(match[3] || '0');

    return hours * 3600 + minutes * 60 + seconds;
  }

  /**
   * Generate SCTE-35 EventStream for ad signaling
   */
  generateSCTE35EventStream(
    events: Array<{
      id: number;
      presentationTime: number;
      duration?: number;
      scte35Data: string; // Base64 encoded SCTE-35
    }>,
    timescale: number = 90000
  ): DASHEventStream {
    return {
      schemeIdUri: 'urn:scte:scte35:2014:xml+bin',
      value: 'scte35',
      timescale,
      events: events.map((e) => ({
        id: e.id,
        presentationTime: e.presentationTime,
        duration: e.duration,
        messageData: e.scte35Data,
      })),
    };
  }

  /**
   * Validate MPD manifest
   */
  validateMPD(content: string, context: ManifestContext): ManifestValidationResult {
    const errors: ManifestValidationError[] = [];
    const warnings: ManifestValidationWarning[] = [];

    // Basic validation - in production use a proper XML parser
    if (!content.includes('<?xml')) {
      errors.push({
        code: 'MISSING_XML_DECLARATION',
        message: 'MPD must start with XML declaration',
      });
    }

    if (!content.includes('<MPD')) {
      errors.push({
        code: 'MISSING_MPD_ELEMENT',
        message: 'MPD must contain MPD root element',
      });
    }

    if (!content.includes('minBufferTime')) {
      errors.push({
        code: 'MISSING_MIN_BUFFER_TIME',
        message: 'MPD must have minBufferTime attribute',
      });
    }

    if (context.mode === 'live') {
      if (!content.includes('type="dynamic"')) {
        errors.push({
          code: 'LIVE_NOT_DYNAMIC',
          message: 'Live MPD must have type="dynamic"',
        });
      }

      if (!content.includes('UTCTiming')) {
        warnings.push({
          code: 'MISSING_UTC_TIMING',
          message: 'Live MPD should include UTCTiming for clock sync',
        });
      }
    } else {
      if (!content.includes('type="static"')) {
        errors.push({
          code: 'VOD_NOT_STATIC',
          message: 'VOD MPD must have type="static"',
        });
      }
    }

    if (!content.includes('<Period')) {
      errors.push({
        code: 'MISSING_PERIOD',
        message: 'MPD must contain at least one Period',
      });
    }

    const valid = errors.length === 0;

    recordPerformanceMetric({
      observedAt: new Date(),
      metricType: 'manifest_invalid',
      metricValue: valid ? 0 : 1,
      requestId: context.sessionId,
      errorCode: valid ? null : 'MANIFEST_INVALID',
      errorMessage: valid ? null : (errors[0]?.message ?? 'Manifest validation failed'),
      metadata: {
        source: 'server_validator',
        protocol: context.protocol,
        mode: context.mode,
        container: context.container,
        manifestVersion: context.manifestVersion,
        errorCount: errors.length,
        warningCount: warnings.length,
        errorCodes: errors.slice(0, 20).map((e) => e.code),
      },
    });

    return {
      valid,
      errors,
      warnings,
    };
  }
}

// Export singleton instance
export const dashManifestGenerator = new DASHManifestGenerator();
