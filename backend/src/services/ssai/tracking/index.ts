/**
 * SSAI Tracking Module
 *
 * Event tracking, reconciliation, and audit bundles for SSAI.
 * Provides:
 * - EventTracker - Impression/quartile/complete tracking with pixel firing
 * - DedupeLayer - Exactly-once semantics for tracking events
 * - VODReconciler - Deterministic VOD timeline reconciliation
 * - LiveReconciler - Live drift-tolerant reconciliation
 * - AuditBundleGenerator - Cryptographically signed audit bundles
 * - TrackingRateLimiter - Rate limiting for tracking events
 */

export const TRACKING_MODULE_VERSION = '1.0.0';

// Types
export * from './types';

// Event Tracker
export { EventTracker, EventTrackerConfig } from './EventTracker';

// Deduplication
export { DedupeLayer, DedupeResult, CrossSourceReconciler } from './DedupeLayer';

// VOD Reconciliation
export { VODReconciler, VODReconcilerConfig } from './VODReconciler';

// Live Reconciliation
export { LiveReconciler, LiveReconcilerConfig } from './LiveReconciler';

// Audit Bundles
export {
  AuditBundleGenerator,
  AuditBundleGeneratorConfig,
  TrackingRateLimiter,
} from './AuditBundleGenerator';

export interface AuditBundle {
  bundleId: string;
  sessionId: string;
  timeRange: {
    start: Date;
    end: Date;
  };
  decisionTrace: unknown[];
  manifestVersions: number[];
  cueDetails: unknown[];
  creatives: unknown[];
  events: TrackingEvent[];
  signature: string;
  generatedAt: Date;
}
