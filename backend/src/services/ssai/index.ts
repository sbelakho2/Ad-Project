/**
 * SSAI Service - Server-Side Ad Insertion
 *
 * This module provides comprehensive SSAI functionality for Live + VOD,
 * HLS + DASH, TS + CMAF delivery.
 *
 * @module ssai
 */

// Session Management
export * from './session';

// Manifest Generation (excluding types that conflict with signaling)
export {
  HLSManifestGenerator,
  DASHManifestGenerator,
  ContainerHandler,
  CachePolicy,
  ManifestService,
} from './manifest';

// Re-export manifest types (primary location for manifest types)
export type {
  DASHPeriod,
  DASHAdaptationSet,
  DASHRepresentation,
  DASHSegmentTemplate,
  DASHSegmentList,
  DASHEventStream as ManifestDASHEventStream,
  DASHEvent as ManifestDASHEvent,
  ManifestContext,
  ManifestProtocol,
  ContainerFormat,
  StreamMode,
  SegmentInfo,
  HLSManifestOptions,
  DASHManifestOptions,
  ManifestCachePolicy,
} from './manifest/types';

// Break Signaling (primary location for signaling types)
export * from './signaling';

// Pod Planning & Decisioning
export * from './pod';

// Stitching & Packaging
export * from './stitching';

// Tracking & Reconciliation
export * from './tracking';

// Operability & Reliability
export * from './operability';
