/**
 * SSAI Manifest Parser for Web SDK
 *
 * Parses HLS and DASH manifests to detect ad breaks and track progress.
 */

import { SSAIAdBreak, SSAIAd } from './types';

/**
 * HLS EXT-X-DATERANGE parsed data
 */
interface DateRangeCue {
  id: string;
  class: string;
  startDate: Date;
  endDate?: Date;
  duration?: number;
  plannedDuration?: number;
  scte35Cmd?: string;
  scte35Out?: string;
  scte35In?: string;
  customAttributes: Record<string, string>;
}

/**
 * DASH Event parsed data
 */
interface DASHEvent {
  id: string;
  schemeIdUri: string;
  presentationTime: number;
  duration?: number;
  messageData?: string;
}

export class SSAIManifestParser {
  private adBreaks: Map<string, SSAIAdBreak> = new Map();
  private lastManifestVersion: string | null = null;

  /**
   * Parse HLS manifest for ad cues
   */
  parseHLSManifest(manifestContent: string): SSAIAdBreak[] {
    const lines = manifestContent.split('\n');
    const dateRanges: DateRangeCue[] = [];

    for (const line of lines) {
      if (line.startsWith('#EXT-X-DATERANGE:')) {
        const cue = this.parseDateRange(line);
        if (cue) {
          dateRanges.push(cue);
        }
      }
    }

    // Convert date ranges to ad breaks
    return this.dateRangesToAdBreaks(dateRanges);
  }

  /**
   * Parse EXT-X-DATERANGE tag
   */
  private parseDateRange(line: string): DateRangeCue | null {
    const content = line.substring('#EXT-X-DATERANGE:'.length);
    const attributes = this.parseAttributes(content);

    const id = attributes['ID'];
    const className = attributes['CLASS'] || '';
    const startDateStr = attributes['START-DATE'];

    if (!id || !startDateStr) return null;

    const cue: DateRangeCue = {
      id,
      class: className,
      startDate: new Date(startDateStr),
      customAttributes: {},
    };

    if (attributes['END-DATE']) {
      cue.endDate = new Date(attributes['END-DATE']);
    }

    if (attributes['DURATION']) {
      cue.duration = parseFloat(attributes['DURATION']);
    }

    if (attributes['PLANNED-DURATION']) {
      cue.plannedDuration = parseFloat(attributes['PLANNED-DURATION']);
    }

    if (attributes['SCTE35-CMD']) {
      cue.scte35Cmd = attributes['SCTE35-CMD'];
    }

    if (attributes['SCTE35-OUT']) {
      cue.scte35Out = attributes['SCTE35-OUT'];
    }

    if (attributes['SCTE35-IN']) {
      cue.scte35In = attributes['SCTE35-IN'];
    }

    // Collect custom X- attributes
    for (const [key, value] of Object.entries(attributes)) {
      if (key.startsWith('X-')) {
        cue.customAttributes[key] = value;
      }
    }

    return cue;
  }

  /**
   * Parse attributes from an HLS tag
   */
  private parseAttributes(content: string): Record<string, string> {
    const attributes: Record<string, string> = {};
    const regex = /([A-Z0-9-]+)=(?:"([^"]*)"|([^,]*))/g;
    let match;

    while ((match = regex.exec(content)) !== null) {
      const key = match[1];
      const value = match[2] ?? match[3];
      attributes[key] = value;
    }

    return attributes;
  }

  /**
   * Convert date range cues to ad breaks
   */
  private dateRangesToAdBreaks(cues: DateRangeCue[]): SSAIAdBreak[] {
    const breaks: SSAIAdBreak[] = [];
    let currentBreak: SSAIAdBreak | null = null;
    let breakPosition = 0;

    for (const cue of cues) {
      // Check for ad break start
      if (
        cue.scte35Out ||
        cue.class?.includes('ad-break') ||
        cue.class?.includes('com.apple.hls.interstitial')
      ) {
        currentBreak = {
          podId: cue.id,
          position: breakPosition,
          duration: cue.duration || cue.plannedDuration || 0,
          adCount: 0,
          type: this.determineBreakType(breakPosition),
          ads: [],
        };
        breaks.push(currentBreak);
      }

      // Check for individual ad
      if (cue.class?.includes('ad') || cue.customAttributes['X-ASSET-ID']) {
        const ad: SSAIAd = {
          slotId: cue.customAttributes['X-SLOT-ID'] || cue.id,
          creativeId: cue.customAttributes['X-ASSET-ID'] || cue.id,
          duration: cue.duration || 0,
          advertiser: cue.customAttributes['X-ADVERTISER'],
          title: cue.customAttributes['X-TITLE'],
          clickThroughUrl: cue.customAttributes['X-CLICK-THROUGH'],
        };

        if (cue.customAttributes['X-SKIP-OFFSET']) {
          ad.skipOffset = parseFloat(cue.customAttributes['X-SKIP-OFFSET']);
        }

        if (currentBreak) {
          currentBreak.ads.push(ad);
          currentBreak.adCount = currentBreak.ads.length;
          currentBreak.duration += ad.duration;
        }
      }

      // Check for ad break end
      if (cue.scte35In || cue.class?.includes('ad-break-end')) {
        currentBreak = null;
      }

      breakPosition += cue.duration || 0;
    }

    return breaks;
  }

  /**
   * Parse DASH manifest (MPD) for ad events
   */
  parseDASHManifest(mpdContent: string): SSAIAdBreak[] {
    // Parse XML
    const parser = new DOMParser();
    const doc = parser.parseFromString(mpdContent, 'text/xml');

    const breaks: SSAIAdBreak[] = [];

    // Look for EventStream elements
    const eventStreams = doc.querySelectorAll('EventStream');

    for (const eventStream of Array.from(eventStreams)) {
      const schemeIdUri = eventStream.getAttribute('schemeIdUri') || '';

      // SCTE-35 event stream
      if (schemeIdUri.includes('scte35') || schemeIdUri.includes('urn:scte:scte35')) {
        const events = eventStream.querySelectorAll('Event');

        for (const event of Array.from(events)) {
          const eventData = this.parseDASHEvent(event, schemeIdUri);
          if (eventData) {
            const adBreak = this.eventToAdBreak(eventData);
            if (adBreak) {
              breaks.push(adBreak);
            }
          }
        }
      }
    }

    // Look for Period elements with ad info
    const periods = doc.querySelectorAll('Period');
    let periodStart = 0;

    for (const period of Array.from(periods)) {
      const id = period.getAttribute('id') || '';
      const durationStr = period.getAttribute('duration');

      // Check for ad period markers
      if (id.includes('ad') || id.includes('pod')) {
        const duration = durationStr ? this.parseDuration(durationStr) : 0;

        const adBreak: SSAIAdBreak = {
          podId: id,
          position: periodStart,
          duration,
          adCount: 1,
          type: this.determineBreakType(periodStart),
          ads: [
            {
              slotId: id,
              creativeId: id,
              duration,
            },
          ],
        };

        breaks.push(adBreak);
      }

      if (durationStr) {
        periodStart += this.parseDuration(durationStr);
      }
    }

    return breaks;
  }

  /**
   * Parse a DASH Event element
   */
  private parseDASHEvent(
    event: Element,
    schemeIdUri: string
  ): DASHEvent | null {
    const id = event.getAttribute('id') || '';
    const presentationTimeStr = event.getAttribute('presentationTime') || '0';
    const durationStr = event.getAttribute('duration');

    const dashEvent: DASHEvent = {
      id,
      schemeIdUri,
      presentationTime: parseFloat(presentationTimeStr),
      duration: durationStr ? parseFloat(durationStr) : undefined,
      messageData: event.textContent || undefined,
    };

    return dashEvent;
  }

  /**
   * Convert DASH event to ad break
   */
  private eventToAdBreak(event: DASHEvent): SSAIAdBreak | null {
    if (!event.duration) return null;

    return {
      podId: event.id,
      position: event.presentationTime,
      duration: event.duration,
      adCount: 1,
      type: this.determineBreakType(event.presentationTime),
      ads: [
        {
          slotId: event.id,
          creativeId: event.id,
          duration: event.duration,
        },
      ],
    };
  }

  /**
   * Parse ISO 8601 duration
   */
  private parseDuration(durationStr: string): number {
    // PT#H#M#S format
    const match = durationStr.match(
      /PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?/
    );

    if (!match) return 0;

    const hours = parseFloat(match[1] || '0');
    const minutes = parseFloat(match[2] || '0');
    const seconds = parseFloat(match[3] || '0');

    return hours * 3600 + minutes * 60 + seconds;
  }

  /**
   * Determine break type based on position
   */
  private determineBreakType(
    position: number
  ): 'preroll' | 'midroll' | 'postroll' {
    if (position === 0) return 'preroll';
    // We can't reliably detect postroll without duration
    return 'midroll';
  }

  /**
   * Get all detected ad breaks
   */
  getAdBreaks(): SSAIAdBreak[] {
    return Array.from(this.adBreaks.values());
  }

  /**
   * Get ad break at a specific time
   */
  getAdBreakAtTime(timeSeconds: number): SSAIAdBreak | null {
    for (const adBreak of this.adBreaks.values()) {
      const breakEnd = adBreak.position + adBreak.duration;
      if (timeSeconds >= adBreak.position && timeSeconds < breakEnd) {
        return adBreak;
      }
    }
    return null;
  }

  /**
   * Get ad at a specific time within a break
   */
  getAdAtTime(adBreak: SSAIAdBreak, timeSeconds: number): SSAIAd | null {
    let adStart = adBreak.position;

    for (const ad of adBreak.ads) {
      const adEnd = adStart + ad.duration;
      if (timeSeconds >= adStart && timeSeconds < adEnd) {
        return ad;
      }
      adStart = adEnd;
    }

    return null;
  }

  /**
   * Clear cached data
   */
  clear(): void {
    this.adBreaks.clear();
    this.lastManifestVersion = null;
  }
}
