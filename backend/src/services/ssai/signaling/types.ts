/**
 * SSAI Signaling Types
 *
 * Types for SCTE-35 parsing, cue handling, and break signaling.
 */

/**
 * SCTE-35 splice command type
 */
export type SpliceCommandType =
  | 'splice_null'
  | 'splice_insert'
  | 'time_signal'
  | 'bandwidth_reservation'
  | 'private_command';

/**
 * SCTE-35 segmentation type
 */
export type SegmentationType =
  | 'not_indicated'
  | 'content_identification'
  | 'program_start'
  | 'program_end'
  | 'program_early_termination'
  | 'program_breakaway'
  | 'program_resumption'
  | 'program_runover_planned'
  | 'program_runover_unplanned'
  | 'program_overlap_start'
  | 'program_blackout_override'
  | 'program_start_in_progress'
  | 'chapter_start'
  | 'chapter_end'
  | 'break_start'
  | 'break_end'
  | 'opening_credit_start'
  | 'opening_credit_end'
  | 'closing_credit_start'
  | 'closing_credit_end'
  | 'provider_advertisement_start'
  | 'provider_advertisement_end'
  | 'distributor_advertisement_start'
  | 'distributor_advertisement_end'
  | 'provider_placement_opportunity_start'
  | 'provider_placement_opportunity_end'
  | 'distributor_placement_opportunity_start'
  | 'distributor_placement_opportunity_end'
  | 'provider_overlay_placement_opportunity_start'
  | 'provider_overlay_placement_opportunity_end'
  | 'distributor_overlay_placement_opportunity_start'
  | 'distributor_overlay_placement_opportunity_end'
  | 'unscheduled_event_start'
  | 'unscheduled_event_end'
  | 'network_start'
  | 'network_end';

/**
 * Segmentation type IDs
 */
export const SEGMENTATION_TYPE_IDS: Record<number, SegmentationType> = {
  0x00: 'not_indicated',
  0x01: 'content_identification',
  0x10: 'program_start',
  0x11: 'program_end',
  0x12: 'program_early_termination',
  0x13: 'program_breakaway',
  0x14: 'program_resumption',
  0x15: 'program_runover_planned',
  0x16: 'program_runover_unplanned',
  0x17: 'program_overlap_start',
  0x18: 'program_blackout_override',
  0x19: 'program_start_in_progress',
  0x20: 'chapter_start',
  0x21: 'chapter_end',
  0x22: 'break_start',
  0x23: 'break_end',
  0x24: 'opening_credit_start',
  0x25: 'opening_credit_end',
  0x26: 'closing_credit_start',
  0x27: 'closing_credit_end',
  0x30: 'provider_advertisement_start',
  0x31: 'provider_advertisement_end',
  0x32: 'distributor_advertisement_start',
  0x33: 'distributor_advertisement_end',
  0x34: 'provider_placement_opportunity_start',
  0x35: 'provider_placement_opportunity_end',
  0x36: 'distributor_placement_opportunity_start',
  0x37: 'distributor_placement_opportunity_end',
  0x38: 'provider_overlay_placement_opportunity_start',
  0x39: 'provider_overlay_placement_opportunity_end',
  0x3a: 'distributor_overlay_placement_opportunity_start',
  0x3b: 'distributor_overlay_placement_opportunity_end',
  0x40: 'unscheduled_event_start',
  0x41: 'unscheduled_event_end',
  0x50: 'network_start',
  0x51: 'network_end',
};

/**
 * Parsed SCTE-35 message
 */
export interface SCTE35Message {
  /** Table ID (should be 0xFC) */
  tableId: number;

  /** Protocol version */
  protocolVersion: number;

  /** Encrypted packet flag */
  encryptedPacket: boolean;

  /** PTS adjustment */
  ptsAdjustment: number;

  /** CW index */
  cwIndex: number;

  /** Tier */
  tier: number;

  /** Splice command length */
  spliceCommandLength: number;

  /** Splice command type */
  spliceCommandType: SpliceCommandType;

  /** Splice command */
  spliceCommand?: SpliceInsert | TimeSignal;

  /** Descriptor loop length */
  descriptorLoopLength: number;

  /** Splice descriptors */
  descriptors: SpliceDescriptor[];

  /** CRC32 */
  crc32?: number;

  /** Raw base64 data */
  rawData: string;
}

/**
 * Splice Insert command
 */
export interface SpliceInsert {
  /** Splice event ID */
  spliceEventId: number;

  /** Splice event cancel indicator */
  spliceEventCancelIndicator: boolean;

  /** Out of network indicator */
  outOfNetworkIndicator?: boolean;

  /** Program splice flag */
  programSpliceFlag?: boolean;

  /** Duration flag */
  durationFlag?: boolean;

  /** Splice immediate flag */
  spliceImmediateFlag?: boolean;

  /** Splice time */
  spliceTime?: SpliceTime;

  /** Break duration */
  breakDuration?: BreakDuration;

  /** Unique program ID */
  uniqueProgramId?: number;

  /** Avail num */
  availNum?: number;

  /** Avails expected */
  availsExpected?: number;
}

/**
 * Time Signal command
 */
export interface TimeSignal {
  /** Splice time */
  spliceTime: SpliceTime;
}

/**
 * Splice time
 */
export interface SpliceTime {
  /** Time specified flag */
  timeSpecifiedFlag: boolean;

  /** PTS time (90kHz) */
  ptsTime?: number;
}

/**
 * Break duration
 */
export interface BreakDuration {
  /** Auto return flag */
  autoReturn: boolean;

  /** Duration (90kHz) */
  duration: number;
}

/**
 * Splice descriptor
 */
export interface SpliceDescriptor {
  /** Descriptor tag */
  tag: number;

  /** Descriptor length */
  length: number;

  /** Identifier */
  identifier: string;

  /** Descriptor data */
  data: Uint8Array;

  /** Parsed segmentation descriptor (if applicable) */
  segmentation?: SegmentationDescriptor;
}

/**
 * Segmentation descriptor (0x02)
 */
export interface SegmentationDescriptor {
  /** Segmentation event ID */
  segmentationEventId: number;

  /** Segmentation event cancel indicator */
  segmentationEventCancelIndicator: boolean;

  /** Program segmentation flag */
  programSegmentationFlag?: boolean;

  /** Segmentation duration flag */
  segmentationDurationFlag?: boolean;

  /** Delivery not restricted flag */
  deliveryNotRestrictedFlag?: boolean;

  /** Web delivery allowed flag */
  webDeliveryAllowedFlag?: boolean;

  /** No regional blackout flag */
  noRegionalBlackoutFlag?: boolean;

  /** Archive allowed flag */
  archiveAllowedFlag?: boolean;

  /** Device restrictions */
  deviceRestrictions?: number;

  /** Segmentation duration (90kHz) */
  segmentationDuration?: number;

  /** Segmentation UPID type */
  segmentationUpidType?: number;

  /** Segmentation UPID */
  segmentationUpid?: Uint8Array;

  /** Segmentation type ID */
  segmentationTypeId: number;

  /** Segmentation type name */
  segmentationType: SegmentationType;

  /** Segment num */
  segmentNum?: number;

  /** Segments expected */
  segmentsExpected?: number;

  /** Sub-segment num */
  subSegmentNum?: number;

  /** Sub-segments expected */
  subSegmentsExpected?: number;
}

/**
 * Parsed HLS DATERANGE cue
 */
export interface HLSDateRangeCue {
  /** Unique ID */
  id: string;

  /** Start date (ISO 8601) */
  startDate: Date;

  /** End date (ISO 8601) */
  endDate?: Date;

  /** Duration in seconds */
  duration?: number;

  /** Planned duration in seconds */
  plannedDuration?: number;

  /** Class */
  class?: string;

  /** SCTE-35 command (hex) */
  scte35Cmd?: string;

  /** SCTE-35 out signal (hex) */
  scte35Out?: string;

  /** SCTE-35 in signal (hex) */
  scte35In?: string;

  /** End on next flag */
  endOnNext?: boolean;

  /** Custom X- attributes */
  customAttributes: Record<string, string | number>;
}

/**
 * Parsed DASH Event
 */
export interface DASHEventCue {
  /** Event ID */
  id?: number;

  /** Presentation time */
  presentationTime: number;

  /** Timescale */
  timescale: number;

  /** Duration */
  duration?: number;

  /** Scheme ID URI */
  schemeIdUri: string;

  /** Value */
  value?: string;

  /** Message data */
  messageData?: string;

  /** Parsed SCTE-35 (if applicable) */
  scte35?: SCTE35Message;
}

/**
 * Normalized ad break cue
 */
export interface AdBreakCue {
  /** Unique cue ID */
  cueId: string;

  /** Cue type */
  type: 'start' | 'end';

  /** Media time in seconds */
  mediaTime: number;

  /** Wall clock time (if available) */
  wallClockTime?: Date;

  /** Break duration in seconds (if known) */
  duration?: number;

  /** Planned duration in seconds */
  plannedDuration?: number;

  /** Associated break ID */
  breakId?: string;

  /** Auto return flag */
  autoReturn?: boolean;

  /** Avail num */
  availNum?: number;

  /** Avails expected */
  availsExpected?: number;

  /** SCTE-35 segmentation type */
  segmentationType?: SegmentationType;

  /** Raw SCTE-35 data */
  rawScte35?: string;

  /** Source (scte35 | daterange | dash_event | schedule) */
  source: 'scte35' | 'daterange' | 'dash_event' | 'schedule';

  /** Original parsed cue */
  original?: SCTE35Message | HLSDateRangeCue | DASHEventCue;
}

/**
 * Cue deduplication key
 */
export interface CueDedupeKey {
  /** Cue ID or event ID */
  cueId: string;

  /** Media time window (rounded to nearest second) */
  timeWindow: number;

  /** Source */
  source: string;
}

/**
 * Pod slot
 */
export interface PodSlot {
  /** Slot ID */
  slotId: string;

  /** Slot index (0-based) */
  index: number;

  /** Duration in seconds */
  duration: number;

  /** Creative ID (when filled) */
  creativeId?: string;

  /** Status */
  status: 'pending' | 'filled' | 'slate' | 'skipped';
}

/**
 * Normalized pod plan
 */
export interface PodPlan {
  /** Break ID */
  breakId: string;

  /** Pod ID */
  podId: string;

  /** Start time (wall clock) */
  startTime: Date;

  /** Start time (media time in seconds) */
  mediaStartTime: number;

  /** Planned duration in seconds */
  plannedDuration: number;

  /** Actual duration in seconds */
  actualDuration?: number;

  /** Slots in this pod */
  slots: PodSlot[];

  /** Status */
  status: 'planned' | 'active' | 'completed' | 'cancelled';

  /** Start cue */
  startCue: AdBreakCue;

  /** End cue (if received) */
  endCue?: AdBreakCue;

  /** Keyframe aligned */
  keyframeAligned: boolean;

  /** Splice point media time */
  splicePointTime: number;
}

/**
 * Drift handling behavior
 */
export type DriftBehavior = 'partial_fill' | 'slate' | 'skip' | 'extend';

/**
 * Drift detection result
 */
export interface DriftDetection {
  /** Drift in seconds (positive = late, negative = early) */
  driftSeconds: number;

  /** Absolute drift */
  absoluteDrift: number;

  /** Drift severity */
  severity: 'none' | 'minor' | 'major' | 'critical';

  /** Recommended behavior */
  recommendedBehavior: DriftBehavior;
}

/**
 * Cue ingestion result
 */
export interface CueIngestionResult {
  /** Whether cue was accepted */
  accepted: boolean;

  /** Normalized cue */
  cue?: AdBreakCue;

  /** Rejection reason */
  rejectionReason?: string;

  /** Whether this was a duplicate */
  duplicate: boolean;

  /** Associated pod plan */
  podPlan?: PodPlan;
}
