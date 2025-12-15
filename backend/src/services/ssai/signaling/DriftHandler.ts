/**
 * Drift Handler
 *
 * Standalone service for handling timing drift between expected and actual
 * ad break positions, including slate fallback behavior.
 */

import { AdBreakCue } from '../signaling/types';

export interface DriftConfig {
  /** Maximum acceptable early arrival (ms) */
  maxEarlyArrivalMs: number;

  /** Maximum acceptable late arrival (ms) */
  maxLateArrivalMs: number;

  /** Threshold to trigger partial fill (ms) */
  partialFillThresholdMs: number;

  /** Threshold to trigger slate fallback (ms) */
  slateFallbackThresholdMs: number;

  /** Enable automatic slate insertion */
  enableAutoSlate: boolean;

  /** Slate asset URL */
  slateUrl?: string;

  /** Slate duration (seconds) */
  slateDuration: number;
}

export interface DriftAnalysis {
  /** Original cue */
  cue: AdBreakCue;

  /** Expected start time (ms) */
  expectedStartMs: number;

  /** Actual arrival time (ms) */
  actualArrivalMs: number;

  /** Drift amount (positive = late, negative = early) */
  driftMs: number;

  /** Drift type */
  driftType: 'none' | 'early' | 'late' | 'severe';

  /** Recommended action */
  action: DriftAction;

  /** Whether slate should be inserted */
  insertSlate: boolean;

  /** Slate duration if needed */
  slateDurationMs?: number;

  /** Adjusted break duration */
  adjustedDurationMs: number;
}

export type DriftAction =
  | 'proceed'
  | 'partial-fill'
  | 'slate-fill'
  | 'skip'
  | 'defer';

export interface SlateSegment {
  uri: string;
  duration: number;
  sequence: number;
}

/**
 * Drift Handler for managing timing discrepancies in SSAI.
 */
export class DriftHandler {
  private config: DriftConfig;
  private slateSegments: SlateSegment[] = [];

  constructor(config?: Partial<DriftConfig>) {
    this.config = {
      maxEarlyArrivalMs: 5000, // 5 seconds early
      maxLateArrivalMs: 10000, // 10 seconds late
      partialFillThresholdMs: 3000, // 3 seconds
      slateFallbackThresholdMs: 2000, // 2 seconds
      enableAutoSlate: true,
      slateDuration: 15, // 15 seconds
      ...config,
    };
  }

  /**
   * Analyze drift for a cue arrival.
   */
  analyzeDrift(
    cue: AdBreakCue,
    expectedStartMs: number,
    actualArrivalMs: number
  ): DriftAnalysis {
    const driftMs = actualArrivalMs - expectedStartMs;
    const absDrift = Math.abs(driftMs);

    // Determine drift type
    let driftType: DriftAnalysis['driftType'];
    if (absDrift < 500) {
      driftType = 'none';
    } else if (driftMs < 0) {
      driftType = 'early';
    } else if (driftMs > this.config.maxLateArrivalMs) {
      driftType = 'severe';
    } else {
      driftType = 'late';
    }

    // Determine action
    const action = this.determineAction(driftMs, cue.duration);

    // Determine if slate is needed
    const insertSlate =
      this.config.enableAutoSlate &&
      (action === 'slate-fill' || driftType === 'severe');

    // Calculate slate duration if needed
    let slateDurationMs: number | undefined;
    if (insertSlate) {
      slateDurationMs = this.calculateSlateDuration(driftMs, cue.duration);
    }

    // Calculate adjusted duration
    const adjustedDurationMs = this.calculateAdjustedDuration(
      cue.duration,
      driftMs,
      action
    );

    return {
      cue,
      expectedStartMs,
      actualArrivalMs,
      driftMs,
      driftType,
      action,
      insertSlate,
      slateDurationMs,
      adjustedDurationMs,
    };
  }

  /**
   * Determine the appropriate action for drift.
   */
  private determineAction(driftMs: number, breakDurationMs: number): DriftAction {
    const absDrift = Math.abs(driftMs);

    // No significant drift
    if (absDrift < 500) {
      return 'proceed';
    }

    // Early arrival - can proceed but may need to defer
    if (driftMs < 0) {
      if (absDrift > this.config.maxEarlyArrivalMs) {
        return 'defer';
      }
      return 'proceed';
    }

    // Late arrival
    if (driftMs > this.config.maxLateArrivalMs) {
      // Too late - skip the break
      return 'skip';
    }

    if (driftMs > this.config.slateFallbackThresholdMs) {
      // Need slate to fill gap
      return 'slate-fill';
    }

    if (driftMs > this.config.partialFillThresholdMs) {
      // Can partially fill with shorter ads
      return 'partial-fill';
    }

    return 'proceed';
  }

  /**
   * Calculate slate duration to fill drift gap.
   */
  private calculateSlateDuration(driftMs: number, breakDurationMs: number): number {
    // Slate should fill the gap until ads can start
    const gapMs = Math.max(0, driftMs);

    // Cap at break duration
    return Math.min(gapMs, breakDurationMs);
  }

  /**
   * Calculate adjusted break duration after accounting for drift.
   */
  private calculateAdjustedDuration(
    originalDurationMs: number,
    driftMs: number,
    action: DriftAction
  ): number {
    switch (action) {
      case 'proceed':
        return originalDurationMs;

      case 'partial-fill':
        // Reduce duration by late amount
        return Math.max(0, originalDurationMs - driftMs);

      case 'slate-fill':
        // Full duration, but slate fills part
        return originalDurationMs;

      case 'skip':
        return 0;

      case 'defer':
        return originalDurationMs;

      default:
        return originalDurationMs;
    }
  }

  /**
   * Generate slate segments to fill a duration.
   */
  generateSlateSegments(durationMs: number): SlateSegment[] {
    if (!this.config.slateUrl) {
      return [];
    }

    const segments: SlateSegment[] = [];
    const segmentDuration = this.config.slateDuration;
    const totalDurationSec = durationMs / 1000;

    let remaining = totalDurationSec;
    let sequence = 0;

    while (remaining > 0) {
      const duration = Math.min(segmentDuration, remaining);
      segments.push({
        uri: this.config.slateUrl,
        duration,
        sequence: sequence++,
      });
      remaining -= duration;
    }

    return segments;
  }

  /**
   * Configure slate URL.
   */
  setSlateUrl(url: string): void {
    this.config.slateUrl = url;
  }

  /**
   * Check if slate is configured.
   */
  hasSlate(): boolean {
    return !!this.config.slateUrl;
  }

  /**
   * Get configuration.
   */
  getConfig(): DriftConfig {
    return { ...this.config };
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<DriftConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Handle early cue arrival.
   */
  handleEarlyArrival(cue: AdBreakCue, earlyMs: number): DriftAnalysis {
    const expectedStartMs = Date.now();
    const actualArrivalMs = expectedStartMs - earlyMs;

    return this.analyzeDrift(cue, expectedStartMs, actualArrivalMs);
  }

  /**
   * Handle late cue arrival.
   */
  handleLateArrival(cue: AdBreakCue, lateMs: number): DriftAnalysis {
    const expectedStartMs = Date.now() - lateMs;
    const actualArrivalMs = Date.now();

    return this.analyzeDrift(cue, expectedStartMs, actualArrivalMs);
  }
}

export default DriftHandler;
