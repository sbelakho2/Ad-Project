/**
 * DASH Event Handler
 *
 * Parses MPEG-DASH Events and EventStream for SCTE-35 signaling.
 * Follows DVB-DASH guidance for SCTE-214 encapsulation.
 */

import { AdBreakCue, CueType, SegmentationType } from './types';
import { SCTE35Parser } from './SCTE35Parser';

/**
 * Simplified SCTE35 message for DASH XML parsing
 */
interface ParsedSCTE35 {
  tableId: number;
  commandType: 'splice_insert' | 'time_signal' | 'splice_null';
  ptsTime?: number;
  duration?: number;
  segmentationDescriptors?: Array<{
    segmentationTypeId: number;
    segmentationType?: SegmentationType;
    upid?: string;
  }>;
}

export interface DASHEvent {
  /** Event ID */
  id: string;

  /** Event presentation time (in timescale units) */
  presentationTime: number;

  /** Event duration (in timescale units, 0xFFFFFFFF = unknown) */
  duration: number;

  /** Message data (may be text or base64 binary) */
  messageData?: string;

  /** Content encoding (e.g., 'base64') */
  contentEncoding?: string;
}

export interface EventStream {
  /** Scheme ID URI (identifies the signaling type) */
  schemeIdUri: string;

  /** Value (additional scheme-specific identifier) */
  value?: string;

  /** Timescale (ticks per second) */
  timescale: number;

  /** Events in this stream */
  events: DASHEvent[];

  /** Period ID this EventStream belongs to */
  periodId?: string;
}

export interface DASHCueResult {
  cues: AdBreakCue[];
  errors: string[];
}

/**
 * Known SCTE-35 scheme URIs per various specifications.
 */
const SCTE35_SCHEME_URIS = [
  // SCTE-214 Part 1
  'urn:scte:scte35:2013:bin',
  'urn:scte:scte35:2014:bin',
  // SCTE-214 Part 2 (XML form)
  'urn:scte:scte35:2013:xml',
  'urn:scte:scte35:2014:xml',
  // CableLabs
  'urn:cablelabs:md:xsd:signaling:3.0:signaling',
  // Some encoders use these
  'http://www.scte.org/schemas/35/2016',
];

/**
 * Ad marker scheme URIs (non-SCTE35).
 */
const AD_MARKER_SCHEME_URIS = [
  // DASH-IF IOP
  'urn:mpeg:dash:event:2012',
  // CableLabs
  'urn:cablelabs:md:xsd:signaling:3.0',
  // Custom ad markers
  'urn:com:example:adbreak',
];

/**
 * DASH Event Handler for parsing MPEG-DASH Events and EventStream.
 */
export class DASHEventHandler {
  private scte35Parser: SCTE35Parser;

  constructor() {
    this.scte35Parser = new SCTE35Parser();
  }

  /**
   * Parse EventStream from DASH MPD and extract ad cues.
   */
  parseEventStream(eventStream: EventStream): DASHCueResult {
    const cues: AdBreakCue[] = [];
    const errors: string[] = [];

    // Check if this is an SCTE-35 EventStream
    const isSCTE35 = this.isSCTE35Scheme(eventStream.schemeIdUri);
    const isAdMarker = this.isAdMarkerScheme(eventStream.schemeIdUri);

    if (!isSCTE35 && !isAdMarker) {
      // Unknown scheme - skip silently
      return { cues, errors };
    }

    for (const event of eventStream.events) {
      try {
        const cue = this.parseEvent(event, eventStream);
        if (cue) {
          cues.push(cue);
        }
      } catch (error) {
        errors.push(
          `Failed to parse event ${event.id}: ${error instanceof Error ? error.message : String(error)}`
        );
      }
    }

    return { cues, errors };
  }

  /**
   * Parse a single DASH Event into an ad cue.
   */
  private parseEvent(event: DASHEvent, eventStream: EventStream): AdBreakCue | null {
    const presentationTimeSeconds =
      event.presentationTime / eventStream.timescale;
    const durationSeconds =
      event.duration === 0xffffffff
        ? undefined
        : event.duration / eventStream.timescale;

    // Parse SCTE-35 message if present
    let scte35Message: ParsedSCTE35 | undefined;
    let cueType: CueType = 'scte35';

    if (this.isSCTE35Scheme(eventStream.schemeIdUri) && event.messageData) {
      // Determine if binary or XML
      if (eventStream.schemeIdUri.includes(':bin')) {
        // Binary SCTE-35
        const base64Data =
          event.contentEncoding === 'base64'
            ? event.messageData
            : Buffer.from(event.messageData, 'utf8').toString('base64');

        const parsed = this.scte35Parser.parse(base64Data);
        scte35Message = this.convertToSimplified(parsed);
      } else if (eventStream.schemeIdUri.includes(':xml')) {
        // XML SCTE-35 - parse as best effort
        scte35Message = this.parseSCTE35XML(event.messageData);
      }

      if (!scte35Message) {
        // Failed to parse SCTE-35 - skip this event
        return null;
      }
    } else if (this.isAdMarkerScheme(eventStream.schemeIdUri)) {
      cueType = 'daterange';
    }

    // Determine if this is a start or end cue
    const isEndCue = this.isEndCue(scte35Message);

    // Generate cue ID
    const cueId = this.generateCueId(event, eventStream);

    return {
      cueId: cueId,
      type: isEndCue ? 'end' : 'start',
      mediaTime: presentationTimeSeconds,
      duration: durationSeconds,
      plannedDuration: durationSeconds,
      source: this.isSCTE35Scheme(eventStream.schemeIdUri) ? 'scte35' : 'dash_event',
      breakId: this.extractBreakId(event, scte35Message),
      segmentationType:
        scte35Message?.segmentationDescriptors?.[0]?.segmentationType,
      rawScte35: event.messageData,
    };
  }

  /**
   * Convert full SCTE35Message to simplified ParsedSCTE35
   */
  private convertToSimplified(message: ReturnType<typeof this.scte35Parser.parse>): ParsedSCTE35 {
    return {
      tableId: message.tableId,
      commandType: message.spliceCommandType as 'splice_insert' | 'time_signal' | 'splice_null',
      ptsTime: message.ptsAdjustment,
      duration: message.spliceCommand && 'breakDuration' in message.spliceCommand 
        ? message.spliceCommand.breakDuration?.duration 
        : undefined,
      segmentationDescriptors: message.descriptors
        ?.filter(d => d.segmentation)
        .map(d => ({
          segmentationTypeId: d.segmentation!.segmentationTypeId,
          segmentationType: this.mapSegmentationType(d.segmentation!.segmentationTypeId),
          upid: d.segmentation?.segmentationUpid 
            ? Buffer.from(d.segmentation.segmentationUpid).toString('utf8')
            : undefined,
        })),
    };
  }

  /**
   * Parse SCTE-35 XML format (simplified).
   */
  private parseSCTE35XML(xmlData: string): ParsedSCTE35 | undefined {
    // Simplified XML parsing - extract key fields
    try {
      // Look for SpliceTime or BreakDuration
      const ptsMatch = xmlData.match(/pts_time="(\d+)"/i);
      const durationMatch = xmlData.match(/duration="(\d+)"/i);
      const typeIdMatch = xmlData.match(/segmentation_type_id="(\d+)"/i);

      const ptsTime = ptsMatch ? parseInt(ptsMatch[1], 10) : 0;
      const duration = durationMatch
        ? parseInt(durationMatch[1], 10)
        : undefined;
      const segmentationTypeId = typeIdMatch
        ? parseInt(typeIdMatch[1], 10)
        : undefined;

      // Determine command type from XML structure
      let commandType: 'splice_insert' | 'time_signal' | 'splice_null' =
        'splice_null';
      if (xmlData.includes('SpliceInsert') || xmlData.includes('splice_insert')) {
        commandType = 'splice_insert';
      } else if (
        xmlData.includes('TimeSignal') ||
        xmlData.includes('time_signal')
      ) {
        commandType = 'time_signal';
      }

      return {
        tableId: 0xfc,
        commandType,
        ptsTime,
        duration,
        segmentationDescriptors: segmentationTypeId
          ? [
              {
                segmentationTypeId,
                segmentationType: this.mapSegmentationType(segmentationTypeId),
              },
            ]
          : undefined,
      };
    } catch {
      return undefined;
    }
  }

  /**
   * Map segmentation type ID to type string.
   */
  private mapSegmentationType(
    typeId: number
  ): SegmentationType | undefined {
    switch (typeId) {
      case 0x30:
      case 0x32:
      case 0x34:
        return 'provider_placement_opportunity_start';
      case 0x31:
      case 0x33:
      case 0x35:
        return 'provider_placement_opportunity_end';
      case 0x10:
        return 'program_start';
      case 0x11:
        return 'program_end';
      case 0x20:
        return 'chapter_start';
      case 0x21:
        return 'chapter_end';
      case 0x22:
        return 'break_start';
      case 0x23:
        return 'break_end';
      case 0x36:
        return 'provider_advertisement_start';
      case 0x37:
        return 'provider_advertisement_end';
      default:
        return undefined;
    }
  }

  /**
   * Check if this is an end cue.
   */
  private isEndCue(scte35?: ParsedSCTE35): boolean {
    if (!scte35?.segmentationDescriptors) {
      return false;
    }

    return scte35.segmentationDescriptors.some(
      (desc) =>
        desc.segmentationType?.includes('_end') ||
        desc.segmentationTypeId === 0x31 ||
        desc.segmentationTypeId === 0x33 ||
        desc.segmentationTypeId === 0x35 ||
        desc.segmentationTypeId === 0x37
    );
  }

  /**
   * Generate a unique cue ID.
   */
  private generateCueId(event: DASHEvent, eventStream: EventStream): string {
    const periodPart = eventStream.periodId || 'main';
    const schemePart = eventStream.schemeIdUri
      .split(':')
      .pop()
      ?.substring(0, 8);
    return `dash-${periodPart}-${schemePart}-${event.id}`;
  }

  /**
   * Extract break ID from event or SCTE-35 message.
   */
  private extractBreakId(event: DASHEvent, scte35?: ParsedSCTE35): string {
    // Try to get from SCTE-35 segmentation UPID
    if (scte35?.segmentationDescriptors?.[0]?.upid) {
      return scte35.segmentationDescriptors[0].upid;
    }

    // Try to get from event ID
    if (event.id) {
      return `break-${event.id}`;
    }

    // Generate from presentation time
    return `break-${event.presentationTime}`;
  }

  /**
   * Check if scheme URI is SCTE-35.
   */
  private isSCTE35Scheme(schemeIdUri: string): boolean {
    return SCTE35_SCHEME_URIS.some(
      (uri) => schemeIdUri.toLowerCase() === uri.toLowerCase()
    );
  }

  /**
   * Check if scheme URI is a generic ad marker.
   */
  private isAdMarkerScheme(schemeIdUri: string): boolean {
    return AD_MARKER_SCHEME_URIS.some(
      (uri) => schemeIdUri.toLowerCase().startsWith(uri.toLowerCase())
    );
  }

  /**
   * Parse EventStream from raw MPD XML string.
   */
  parseEventStreamFromXML(xmlContent: string, periodId?: string): EventStream[] {
    const eventStreams: EventStream[] = [];

    // Simple regex-based parsing (production would use proper XML parser)
    const eventStreamRegex =
      /<EventStream[^>]*schemeIdUri="([^"]*)"[^>]*(?:value="([^"]*)")?[^>]*(?:timescale="(\d+)")?[^>]*>([\s\S]*?)<\/EventStream>/gi;

    let match: RegExpExecArray | null;
    while ((match = eventStreamRegex.exec(xmlContent)) !== null) {
      const [, schemeIdUri, value, timescaleStr, content] = match;
      const timescale = timescaleStr ? parseInt(timescaleStr, 10) : 1;

      const events = this.parseEventsFromXML(content);

      eventStreams.push({
        schemeIdUri,
        value,
        timescale,
        events,
        periodId,
      });
    }

    return eventStreams;
  }

  /**
   * Parse individual Event elements from XML.
   */
  private parseEventsFromXML(xmlContent: string): DASHEvent[] {
    const events: DASHEvent[] = [];

    const eventRegex =
      /<Event[^>]*(?:id="([^"]*)")?[^>]*(?:presentationTime="(\d+)")?[^>]*(?:duration="(\d+)")?[^>]*(?:contentEncoding="([^"]*)")?[^>]*(?:>([^<]*)<\/Event>|\/>)/gi;

    let match: RegExpExecArray | null;
    while ((match = eventRegex.exec(xmlContent)) !== null) {
      const [, id, presentationTimeStr, durationStr, contentEncoding, messageData] =
        match;

      events.push({
        id: id || `event-${events.length}`,
        presentationTime: presentationTimeStr
          ? parseInt(presentationTimeStr, 10)
          : 0,
        duration: durationStr ? parseInt(durationStr, 10) : 0xffffffff,
        messageData: messageData?.trim(),
        contentEncoding,
      });
    }

    return events;
  }

  /**
   * Convert EventStream to emsg boxes for segment embedding.
   *
   * This is used when you need to embed events into segments
   * rather than keeping them in the MPD.
   */
  eventStreamToEmsgData(eventStream: EventStream): Buffer[] {
    const emsgBoxes: Buffer[] = [];

    for (const event of eventStream.events) {
      const emsg = this.createEmsgBox(event, eventStream);
      if (emsg) {
        emsgBoxes.push(emsg);
      }
    }

    return emsgBoxes;
  }

  /**
   * Create an emsg box (ISO BMFF Event Message Box).
   *
   * Simplified implementation - production would need full box construction.
   */
  private createEmsgBox(event: DASHEvent, eventStream: EventStream): Buffer | null {
    try {
      // emsg box structure (version 1):
      // - timescale (4 bytes)
      // - presentation_time_delta (4 bytes)
      // - event_duration (4 bytes)
      // - id (4 bytes)
      // - scheme_id_uri (null-terminated string)
      // - value (null-terminated string)
      // - message_data (remaining bytes)

      const schemeIdUri = Buffer.from(eventStream.schemeIdUri + '\0', 'utf8');
      const value = Buffer.from((eventStream.value || '') + '\0', 'utf8');
      const messageData = event.messageData
        ? Buffer.from(event.messageData, 'utf8')
        : Buffer.alloc(0);

      const bodySize = 16 + schemeIdUri.length + value.length + messageData.length;
      const boxSize = 8 + bodySize; // 8 bytes for size + type

      const buffer = Buffer.alloc(boxSize);
      let offset = 0;

      // Box size (4 bytes)
      buffer.writeUInt32BE(boxSize, offset);
      offset += 4;

      // Box type 'emsg' (4 bytes)
      buffer.write('emsg', offset, 4, 'ascii');
      offset += 4;

      // Timescale (4 bytes)
      buffer.writeUInt32BE(eventStream.timescale, offset);
      offset += 4;

      // Presentation time (4 bytes)
      buffer.writeUInt32BE(event.presentationTime, offset);
      offset += 4;

      // Duration (4 bytes)
      buffer.writeUInt32BE(event.duration, offset);
      offset += 4;

      // ID (4 bytes)
      buffer.writeUInt32BE(parseInt(event.id, 10) || 0, offset);
      offset += 4;

      // Scheme ID URI
      schemeIdUri.copy(buffer, offset);
      offset += schemeIdUri.length;

      // Value
      value.copy(buffer, offset);
      offset += value.length;

      // Message data
      messageData.copy(buffer, offset);

      return buffer;
    } catch {
      return null;
    }
  }
}

export default DASHEventHandler;
