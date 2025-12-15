/**
 * SSAI Tracking Types
 *
 * Types and interfaces for the tracking and reconciliation module.
 */

export type TrackingEventType =
  | 'impression'
  | 'start'
  | 'firstQuartile'
  | 'midpoint'
  | 'thirdQuartile'
  | 'complete'
  | 'error'
  | 'skip'
  | 'mute'
  | 'unmute'
  | 'pause'
  | 'resume'
  | 'fullscreen'
  | 'exitFullscreen'
  | 'click';

export interface TrackingEvent {
  /** Event ID (for deduplication) */
  eventId: string;

  /** Event type */
  type: TrackingEventType;

  /** Session ID */
  sessionId: string;

  /** Pod ID */
  podId: string;

  /** Slot ID */
  slotId: string;

  /** Creative ID */
  creativeId: string;

  /** Timestamp when event occurred */
  timestamp: Date;

  /** Media time when event occurred (seconds) */
  mediaTime: number;

  /** Progress percentage (for quartiles) */
  progress?: number;

  /** Error details (for error events) */
  error?: {
    code: string;
    message: string;
    fatal: boolean;
  };

  /** Source of the event */
  source: 'client' | 'server' | 'inferred';

  /** Additional metadata */
  metadata?: Record<string, unknown>;
}

export interface TrackingResult {
  /** Whether tracking was successful */
  success: boolean;

  /** Event ID */
  eventId: string;

  /** Whether this was a duplicate */
  isDuplicate: boolean;

  /** Tracking URL calls made */
  urlsCalled: number;

  /** Errors during tracking */
  errors: string[];
}

export interface TrackingUrl {
  /** URL to call */
  url: string;

  /** Event type this URL is for */
  eventType: TrackingEventType;

  /** Partner ID */
  partnerId?: string;

  /** Whether this is required */
  required: boolean;
}

export interface CreativeTracking {
  /** Creative ID */
  creativeId: string;

  /** Slot ID */
  slotId: string;

  /** Duration of the creative */
  duration: number;

  /** Tracking URLs by event type */
  trackingUrls: Record<TrackingEventType, TrackingUrl[]>;
}

export interface QuartilePosition {
  /** Quartile event type */
  type: 'firstQuartile' | 'midpoint' | 'thirdQuartile' | 'complete';

  /** Position in seconds */
  position: number;

  /** Progress percentage */
  progress: number;
}

export interface ReconciliationResult {
  /** Pod ID */
  podId: string;

  /** Slot reconciliation results */
  slots: SlotReconciliation[];

  /** Overall fill rate */
  fillRate: number;

  /** Overall completion rate */
  completionRate: number;

  /** Discrepancies found */
  discrepancies: Discrepancy[];
}

export interface SlotReconciliation {
  /** Slot ID */
  slotId: string;

  /** Creative ID */
  creativeId?: string;

  /** Expected events */
  expectedEvents: TrackingEventType[];

  /** Received events */
  receivedEvents: TrackingEventType[];

  /** Missing events */
  missingEvents: TrackingEventType[];

  /** Extra events */
  extraEvents: TrackingEventType[];

  /** Completion status */
  completed: boolean;
}

export interface Discrepancy {
  /** Discrepancy type */
  type: 'missing_event' | 'duplicate_event' | 'timing_drift' | 'unexpected_event';

  /** Event type */
  eventType: TrackingEventType;

  /** Slot ID */
  slotId: string;

  /** Details */
  details: string;

  /** Severity */
  severity: 'info' | 'warning' | 'error';
}

export interface AuditBundle {
  /** Bundle ID */
  bundleId: string;

  /** Session ID */
  sessionId: string;

  /** Pod ID */
  podId: string;

  /** Generated at */
  generatedAt: Date;

  /** Decision trace */
  decision: {
    breakId: string;
    cueReceived: Date;
    podPlanned: Date;
    slotsRequested: number;
    slotsFilled: number;
    decisionTimeMs: number;
  };

  /** Manifest versions served */
  manifestVersions: string[];

  /** Cue details */
  cueDetails: {
    type: string;
    raw?: string;
    normalized: unknown;
  };

  /** Creatives chosen */
  creativesChosen: Array<{
    slotId: string;
    creativeId: string;
    partnerId: string;
    bidPrice?: number;
    reason: string;
  }>;

  /** Event timeline */
  eventTimeline: Array<{
    timestamp: Date;
    eventType: TrackingEventType;
    slotId: string;
    source: string;
  }>;

  /** Signature for verification */
  signature: string;
}

export interface DedupeConfig {
  /** Time window for deduplication (seconds) */
  windowSeconds: number;

  /** Maximum events to track per key */
  maxEventsPerKey: number;
}
