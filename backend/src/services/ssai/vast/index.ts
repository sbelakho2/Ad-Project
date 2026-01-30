/**
 * VAST/VMAP Module Index
 *
 * Main entry point for the VAST/VMAP parsing module.
 * Provides a unified API for ad request handling, parsing,
 * scheduling, and tracking integration with the SSAI pipeline.
 */

// =============================================================================
// Re-exports - Types
// =============================================================================

export {
  // VAST Types
  VASTVersion,
  VASTDocument,
  VASTAd,
  VASTInLine,
  VASTWrapper,
  VASTAdSystem,
  VASTCategory,
  VASTPricing,
  VASTImpression,
  VASTViewableImpression,
  VASTCreative,
  VASTUniversalAdId,
  VASTLinear,
  VASTAdParameters,
  VASTMediaFile,
  VASTMezzanine,
  VASTInteractiveCreativeFile,
  VASTClosedCaptionFile,
  VASTTrackingEvent,
  VASTTrackingEventType,
  VASTVideoClicks,
  VASTClickThrough,
  VASTClickTracking,
  VASTCustomClick,
  VASTIcon,
  VASTStaticResource,
  VASTIconClicks,
  VASTIconClickFallbackImage,
  VASTNonLinear,
  VASTCompanionAds,
  VASTCompanion,
  VASTVerification,
  VASTJavaScriptResource,
  VASTExecutableResource,
  VASTVerificationTrackingEvent,
  VASTExtension,

  // VMAP Types
  VMAPDocument,
  VMAPAdBreak,
  VMAPAdSource,
  VMAPInlineAdSource,
  VMAPRedirectAdSource,
  VMAPCustomAdSource,
  VMAPTimeOffset,
  VMAPTrackingEvent,
  VMAPTrackingEventType,
  VMAPExtension,

  // Configuration Types
  VASTParserConfig,
  DEFAULT_VAST_PARSER_CONFIG,
  VMAPParserConfig,
  DEFAULT_VMAP_PARSER_CONFIG,

  // Result Types
  VASTParseResult,
  VMAPParseResult,
  WrapperChainEntry,
  VASTError,
  VASTWarning,
  VMAPError,
  VMAPWarning,
  VASTErrorCode,
  VMAPErrorCode,

  // Utility Types
  NormalizedMediaFile,
  FlattenedTracking,
  SSAIReadyCreative,
} from './types';

// =============================================================================
// Re-exports - Parsers
// =============================================================================

export { default as VASTParser, normalizeMediaFiles, flattenTracking, extractSSAICreative } from './vastParser';
export { default as VMAPParser, calculateAdBreakTimes, getPreRollBreaks, getMidRollBreaks, getPostRollBreaks, getBreakTrackingUrls, mergeVMAPDocuments } from './vmapParser';
export { default as VASTXMLParser, validateVASTStructure, validateVMAPStructure, MAX_XML_SIZE_BYTES } from './xmlParser';

// =============================================================================
// Re-exports - Scheduler
// =============================================================================

export {
  default as AdPodScheduler,
  ScheduledAdPod,
  ScheduledAd,
  SchedulerConfig,
  DEFAULT_SCHEDULER_CONFIG,
  ScheduleResult,
  SchedulerWarning,
  calculateAdLoad,
  getPodsAtTime,
  getNextPod,
  absoluteToContentTime,
  contentToAbsoluteTime,
  generateTimeline,
} from './adPodScheduler';

// =============================================================================
// Re-exports - Tracking
// =============================================================================

export {
  default as VASTTrackingManager,
  BeaconManager,
  BeaconManagerConfig,
  DEFAULT_BEACON_CONFIG,
  TrackingContext,
  TrackingResult,
  TrackingBatchResult,
  VAST_MACROS,
  formatPlayhead,
  getQuartile,
  buildTrackingContext,
  getAllTrackingUrls,
} from './trackingManager';

// =============================================================================
// Unified Ad Request Handler
// =============================================================================

import VASTParser from './vastParser';
import VMAPParser from './vmapParser';
import AdPodScheduler, { ScheduleResult, SchedulerConfig } from './adPodScheduler';
import VASTTrackingManager, { TrackingContext } from './trackingManager';
import {
  VASTDocument,
  VMAPDocument,
  VASTParseResult,
  VMAPParseResult,
  SSAIReadyCreative,
} from './types';
import logger from '../../../utils/logger';

/**
 * Ad request type
 */
export type AdRequestType = 'vast' | 'vmap' | 'auto';

/**
 * Ad request options
 */
export interface AdRequestOptions {
  /** Request type (auto-detect if not specified) */
  type?: AdRequestType;

  /** Scheduler configuration */
  schedulerConfig?: Partial<SchedulerConfig>;

  /** Content duration (required for scheduling) */
  contentDuration?: number;

  /** Whether to resolve wrapper chains */
  resolveWrappers?: boolean;

  /** Whether to resolve VMAP ad tag URIs */
  resolveVmapAdTags?: boolean;

  /** Custom macros for tracking URLs */
  macros?: Record<string, string>;

  /** Timeout for ad requests (ms) */
  timeoutMs?: number;
}

/**
 * Ad request result
 */
export interface AdRequestResult {
  /** Whether the request was successful */
  success: boolean;

  /** Type of response received */
  type: 'vast' | 'vmap';

  /** Parsed VAST document (if VAST) */
  vastDocument?: VASTDocument;

  /** Parsed VMAP document (if VMAP) */
  vmapDocument?: VMAPDocument;

  /** Scheduled ad pods */
  schedule?: ScheduleResult;

  /** SSAI-ready creatives (flattened from all ads) */
  creatives: SSAIReadyCreative[];

  /** Errors encountered */
  errors: Array<{ code: string; message: string }>;

  /** Warnings */
  warnings: Array<{ code: string; message: string }>;

  /** Processing time in ms */
  processingTimeMs: number;
}

/**
 * Unified Ad Request Handler
 *
 * Provides a single entry point for handling VAST/VMAP ad requests
 * with automatic detection, parsing, scheduling, and creative extraction.
 */
export class AdRequestHandler {
  private vastParser: VASTParser;
  private vmapParser: VMAPParser;
  private scheduler: AdPodScheduler;
  private trackingManager: VASTTrackingManager;

  constructor(options: AdRequestOptions = {}) {
    this.vastParser = new VASTParser({
      resolveWrappers: options.resolveWrappers ?? true,
    });

    this.vmapParser = new VMAPParser({
      resolveVAST: options.resolveVmapAdTags ?? false,
    });

    this.scheduler = new AdPodScheduler({
      contentDuration: options.contentDuration ?? 0,
      ...options.schedulerConfig,
    });

    this.trackingManager = new VASTTrackingManager();
  }

  /**
   * Process an ad response (VAST or VMAP)
   */
  async processAdResponse(
    xml: string,
    options: AdRequestOptions = {}
  ): Promise<AdRequestResult> {
    const startTime = Date.now();
    const errors: Array<{ code: string; message: string }> = [];
    const warnings: Array<{ code: string; message: string }> = [];

    try {
      // Detect type
      const type = options.type === 'auto' || !options.type
        ? this.detectType(xml)
        : options.type;

      if (type === 'vmap') {
        return await this.processVMAP(xml, options, startTime);
      } else {
        return await this.processVAST(xml, options, startTime);
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';

      logger.error('[AdRequestHandler] Processing failed', { error: errorMessage });

      return {
        success: false,
        type: 'vast',
        creatives: [],
        errors: [{ code: 'PROCESSING_ERROR', message: errorMessage }],
        warnings: [],
        processingTimeMs: Date.now() - startTime,
      };
    }
  }

  /**
   * Process a VAST response
   */
  private async processVAST(
    xml: string,
    options: AdRequestOptions,
    startTime: number
  ): Promise<AdRequestResult> {
    const parseResult = await this.vastParser.parse(xml);

    const errors = parseResult.errors.map((e) => ({
      code: String(e.code),
      message: e.message,
    }));

    const warnings = parseResult.warnings.map((w) => ({
      code: w.code,
      message: w.message,
    }));

    if (!parseResult.success || !parseResult.document) {
      return {
        success: false,
        type: 'vast',
        creatives: [],
        errors,
        warnings,
        processingTimeMs: Date.now() - startTime,
      };
    }

    // Extract creatives
    const creatives = this.extractCreatives(parseResult.document);

    // Schedule if content duration is provided
    let schedule: ScheduleResult | undefined;
    if (options.contentDuration && options.contentDuration > 0) {
      this.scheduler = new AdPodScheduler({
        contentDuration: options.contentDuration,
        ...options.schedulerConfig,
      });

      schedule = this.scheduler.scheduleFromVAST(
        [parseResult.document],
        [{ startTime: 0 }] // Default to pre-roll
      );

      warnings.push(
        ...schedule.warnings.map((w) => ({ code: w.code, message: w.message }))
      );
    }

    return {
      success: true,
      type: 'vast',
      vastDocument: parseResult.document,
      schedule,
      creatives,
      errors,
      warnings,
      processingTimeMs: Date.now() - startTime,
    };
  }

  /**
   * Process a VMAP response
   */
  private async processVMAP(
    xml: string,
    options: AdRequestOptions,
    startTime: number
  ): Promise<AdRequestResult> {
    const parseResult = await this.vmapParser.parse(xml);

    const errors = parseResult.errors.map((e) => ({
      code: e.code,
      message: e.message,
    }));

    const warnings = parseResult.warnings.map((w) => ({
      code: w.code,
      message: w.message,
    }));

    if (!parseResult.success || !parseResult.document) {
      return {
        success: false,
        type: 'vmap',
        creatives: [],
        errors,
        warnings,
        processingTimeMs: Date.now() - startTime,
      };
    }

    // Extract creatives from all breaks
    const creatives: SSAIReadyCreative[] = [];
    const vastResponses = new Map<string, VASTDocument>();

    for (const adBreak of parseResult.document.adBreaks) {
      if (adBreak.adSource?.type === 'inline' && adBreak.adSource.vastData) {
        creatives.push(...this.extractCreatives(adBreak.adSource.vastData));

        if (adBreak.breakId) {
          vastResponses.set(adBreak.breakId, adBreak.adSource.vastData);
        }
      } else if (adBreak.adSource?.type === 'redirect' && adBreak.adSource.resolvedVast) {
        creatives.push(...this.extractCreatives(adBreak.adSource.resolvedVast));

        if (adBreak.adSource.adTagUri) {
          vastResponses.set(adBreak.adSource.adTagUri, adBreak.adSource.resolvedVast);
        }
      }
    }

    // Schedule pods
    let schedule: ScheduleResult | undefined;
    if (options.contentDuration && options.contentDuration > 0) {
      this.scheduler = new AdPodScheduler({
        contentDuration: options.contentDuration,
        ...options.schedulerConfig,
      });

      schedule = this.scheduler.scheduleFromVMAP(parseResult.document, vastResponses);

      warnings.push(
        ...schedule.warnings.map((w) => ({ code: w.code, message: w.message }))
      );
    }

    return {
      success: true,
      type: 'vmap',
      vmapDocument: parseResult.document,
      schedule,
      creatives,
      errors,
      warnings,
      processingTimeMs: Date.now() - startTime,
    };
  }

  /**
   * Detect response type from XML
   */
  private detectType(xml: string): 'vast' | 'vmap' {
    const trimmed = xml.trim().toLowerCase();

    if (trimmed.includes('<vmap') || trimmed.includes('vmap:vmap')) {
      return 'vmap';
    }

    return 'vast';
  }

  /**
   * Extract SSAI-ready creatives from a VAST document
   */
  private extractCreatives(vastDoc: VASTDocument): SSAIReadyCreative[] {
    const creatives: SSAIReadyCreative[] = [];
    const { extractSSAICreative } = require('./vastParser');

    for (const ad of vastDoc.ads) {
      const creative = extractSSAICreative(ad);
      if (creative) {
        creatives.push(creative);
      }
    }

    return creatives;
  }

  /**
   * Get the tracking manager instance
   */
  getTrackingManager(): VASTTrackingManager {
    return this.trackingManager;
  }

  /**
   * Get the scheduler instance
   */
  getScheduler(): AdPodScheduler {
    return this.scheduler;
  }
}

// =============================================================================
// Factory Functions
// =============================================================================

/**
 * Create a VAST parser with default configuration
 */
export function createVASTParser(options?: Partial<import('./types').VASTParserConfig>): VASTParser {
  return new VASTParser(options);
}

/**
 * Create a VMAP parser with default configuration
 */
export function createVMAPParser(options?: Partial<import('./types').VMAPParserConfig>): VMAPParser {
  return new VMAPParser(options);
}

/**
 * Create an ad request handler
 */
export function createAdRequestHandler(options?: AdRequestOptions): AdRequestHandler {
  return new AdRequestHandler(options);
}

/**
 * Parse VAST XML string (convenience function)
 */
export async function parseVAST(xml: string): Promise<VASTParseResult> {
  const parser = new VASTParser();
  return parser.parse(xml);
}

/**
 * Parse VMAP XML string (convenience function)
 */
export async function parseVMAP(xml: string): Promise<VMAPParseResult> {
  const parser = new VMAPParser();
  return parser.parse(xml);
}

// =============================================================================
// Default Export
// =============================================================================

export default AdRequestHandler;
