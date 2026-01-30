/**
 * Ad Pod Scheduler
 *
 * Schedules ad pods based on VMAP breaks and VAST responses.
 * Handles:
 * - Pre-roll, mid-roll, post-roll scheduling
 * - Pod duration constraints
 * - Creative selection and fallback
 * - Repeat intervals for recurring breaks
 */

import {
  VMAPDocument,
  VMAPAdBreak,
  VMAPTimeOffset,
  VASTDocument,
  VASTAd,
  SSAIReadyCreative,
} from './types';
import { extractSSAICreative, normalizeMediaFiles } from './vastParser';
import logger from '../../../utils/logger';

// =============================================================================
// Time Precision Utility
// =============================================================================

/**
 * Round a time value to millisecond precision to prevent floating-point drift.
 * All time calculations should use this to ensure consistent precision.
 * @param seconds Time in seconds
 * @returns Time rounded to 3 decimal places (millisecond precision)
 */
function roundToMs(seconds: number): number {
  return Math.round(seconds * 1000) / 1000;
}

// =============================================================================
// Types
// =============================================================================

export interface ScheduledAdPod {
  /** Unique pod ID */
  id: string;

  /** Pod position type */
  position: 'pre-roll' | 'mid-roll' | 'post-roll';

  /** Start time in content timeline (seconds) */
  startTime: number;

  /** Total pod duration (seconds) */
  duration: number;

  /** Ads in this pod */
  ads: ScheduledAd[];

  /** Break ID from VMAP (if applicable) */
  breakId?: string;

  /** Repeat interval (if this pod repeats) */
  repeatInterval?: number;

  /** Tracking URLs for pod-level events */
  tracking: {
    breakStart: string[];
    breakEnd: string[];
    error: string[];
  };
}

export interface ScheduledAd {
  /** Ad ID */
  id: string;

  /** Sequence number in pod */
  sequence: number;

  /** Ad duration (seconds) */
  duration: number;

  /** Selected creative */
  creative: SSAIReadyCreative;

  /** Start time within pod (seconds) */
  podOffset: number;

  /** Source VAST ad */
  sourceAd: VASTAd;
}

export interface SchedulerConfig {
  /** Maximum pod duration (seconds) */
  maxPodDuration: number;

  /** Maximum ads per pod */
  maxAdsPerPod: number;

  /** Minimum ad duration (seconds) */
  minAdDuration: number;

  /** Maximum ad duration (seconds) */
  maxAdDuration: number;

  /** Whether to allow skippable ads */
  allowSkippable: boolean;

  /** Default skip offset (seconds) */
  defaultSkipOffset: number;

  /** Media selection preferences */
  mediaPreferences: {
    preferredDelivery?: 'streaming' | 'progressive';
    preferredWidth?: number;
    preferredHeight?: number;
    preferredBitrate?: number;
    supportedMimeTypes?: string[];
    supportedCodecs?: string[];
  };

  /** Content duration (for percentage-based offsets) */
  contentDuration: number;

  /** Minimum gap between mid-roll pods (seconds) */
  minPodGap: number;
}

export const DEFAULT_SCHEDULER_CONFIG: SchedulerConfig = {
  maxPodDuration: 180, // 3 minutes
  maxAdsPerPod: 5,
  minAdDuration: 5,
  maxAdDuration: 120,
  allowSkippable: true,
  defaultSkipOffset: 5,
  mediaPreferences: {
    preferredDelivery: 'streaming',
    supportedMimeTypes: ['video/mp4', 'application/x-mpegURL', 'application/dash+xml'],
  },
  contentDuration: 0,
  minPodGap: 60, // 1 minute minimum between pods
};

export interface ScheduleResult {
  /** Scheduled ad pods */
  pods: ScheduledAdPod[];

  /** Total ad time (seconds) */
  totalAdTime: number;

  /** Total number of ads */
  totalAds: number;

  /** Warnings during scheduling */
  warnings: SchedulerWarning[];

  /** Schedule metadata */
  metadata: {
    preRollCount: number;
    midRollCount: number;
    postRollCount: number;
    averagePodDuration: number;
    fillRate: number; // Ratio of filled ad slots to total
  };
}

export interface SchedulerWarning {
  code: string;
  message: string;
  context?: Record<string, unknown>;
}

// =============================================================================
// Ad Pod Scheduler
// =============================================================================

export class AdPodScheduler {
  private config: SchedulerConfig;

  constructor(config: Partial<SchedulerConfig> = {}) {
    this.config = { ...DEFAULT_SCHEDULER_CONFIG, ...config };
  }

  /**
   * Schedule ad pods from a VMAP document
   */
  scheduleFromVMAP(
    vmap: VMAPDocument,
    vastResponses: Map<string, VASTDocument>
  ): ScheduleResult {
    const pods: ScheduledAdPod[] = [];
    const warnings: SchedulerWarning[] = [];

    let totalAdTime = 0;
    let totalAds = 0;

    for (const adBreak of vmap.adBreaks) {
      // Get VAST for this break
      let vastDoc: VASTDocument | undefined;

      if (adBreak.adSource?.type === 'inline' && adBreak.adSource.vastData) {
        vastDoc = adBreak.adSource.vastData;
      } else if (adBreak.adSource?.type === 'redirect') {
        const key = adBreak.adSource.adTagUri || adBreak.breakId;
        vastDoc = key ? vastResponses.get(key) : undefined;

        if (!vastDoc && adBreak.adSource.resolvedVast) {
          vastDoc = adBreak.adSource.resolvedVast;
        }
      }

      if (!vastDoc) {
        warnings.push({
          code: 'NO_VAST_FOR_BREAK',
          message: `No VAST document found for break ${adBreak.breakId || 'unknown'}`,
          context: { breakId: adBreak.breakId },
        });
        continue;
      }

      // Calculate start time
      const startTime = this.calculateStartTime(adBreak.timeOffset);

      // Determine position type
      const position = this.getPositionType(adBreak.timeOffset);

      // Create pod from VAST
      const pod = this.createPodFromVAST(
        vastDoc,
        adBreak,
        startTime,
        position,
        warnings
      );

      if (pod && pod.ads.length > 0) {
        pods.push(pod);
        totalAdTime += pod.duration;
        totalAds += pod.ads.length;
      }
    }

    // Sort pods by start time
    pods.sort((a, b) => a.startTime - b.startTime);

    // Handle repeat intervals
    const expandedPods = this.expandRepeatIntervals(pods, warnings);

    // Enforce minimum gap between pods
    const adjustedPods = this.enforceMinimumGap(expandedPods, warnings);

    // Calculate metadata
    const preRollCount = adjustedPods.filter((p) => p.position === 'pre-roll').length;
    const midRollCount = adjustedPods.filter((p) => p.position === 'mid-roll').length;
    const postRollCount = adjustedPods.filter((p) => p.position === 'post-roll').length;

    const avgDuration =
      adjustedPods.length > 0
        ? adjustedPods.reduce((sum, p) => sum + p.duration, 0) / adjustedPods.length
        : 0;

    return {
      pods: adjustedPods,
      totalAdTime,
      totalAds,
      warnings,
      metadata: {
        preRollCount,
        midRollCount,
        postRollCount,
        averagePodDuration: avgDuration,
        fillRate: totalAds > 0 ? 1 : 0, // Simplified fill rate
      },
    };
  }

  /**
   * Schedule ad pods from standalone VAST documents
   */
  scheduleFromVAST(
    vastDocuments: VASTDocument[],
    insertionPoints: Array<{ startTime: number; maxDuration?: number }>
  ): ScheduleResult {
    const pods: ScheduledAdPod[] = [];
    const warnings: SchedulerWarning[] = [];

    let totalAdTime = 0;
    let totalAds = 0;

    for (let i = 0; i < insertionPoints.length; i++) {
      const insertionPoint = insertionPoints[i];
      const vastDoc = vastDocuments[i] || vastDocuments[0]; // Reuse first if not enough

      if (!vastDoc) {
        warnings.push({
          code: 'NO_VAST_FOR_INSERTION',
          message: `No VAST document for insertion point at ${insertionPoint.startTime}s`,
        });
        continue;
      }

      const position = this.getPositionFromTime(insertionPoint.startTime);

      const pod = this.createPodFromVAST(
        vastDoc,
        undefined,
        insertionPoint.startTime,
        position,
        warnings,
        insertionPoint.maxDuration
      );

      if (pod && pod.ads.length > 0) {
        pods.push(pod);
        totalAdTime += pod.duration;
        totalAds += pod.ads.length;
      }
    }

    // Sort and enforce gap
    pods.sort((a, b) => a.startTime - b.startTime);
    const adjustedPods = this.enforceMinimumGap(pods, warnings);

    const preRollCount = adjustedPods.filter((p) => p.position === 'pre-roll').length;
    const midRollCount = adjustedPods.filter((p) => p.position === 'mid-roll').length;
    const postRollCount = adjustedPods.filter((p) => p.position === 'post-roll').length;

    return {
      pods: adjustedPods,
      totalAdTime,
      totalAds,
      warnings,
      metadata: {
        preRollCount,
        midRollCount,
        postRollCount,
        averagePodDuration:
          adjustedPods.length > 0
            ? adjustedPods.reduce((sum, p) => sum + p.duration, 0) / adjustedPods.length
            : 0,
        fillRate: totalAds > 0 ? 1 : 0,
      },
    };
  }

  /**
   * Calculate start time from time offset
   */
  private calculateStartTime(offset: VMAPTimeOffset): number {
    switch (offset.type) {
      case 'start':
        return 0;

      case 'end':
        return this.config.contentDuration;

      case 'time':
        return offset.seconds;

      case 'percentage':
        return (offset.percent / 100) * this.config.contentDuration;

      case 'position':
        // Position-based requires external mapping
        // Default behavior: treat as a fraction of content
        return (offset.position / 10) * this.config.contentDuration;

      default:
        return 0;
    }
  }

  /**
   * Get position type from time offset
   */
  private getPositionType(offset: VMAPTimeOffset): 'pre-roll' | 'mid-roll' | 'post-roll' {
    if (offset.type === 'start') {
      return 'pre-roll';
    }
    if (offset.type === 'end') {
      return 'post-roll';
    }
    if (offset.type === 'time' && offset.seconds === 0) {
      return 'pre-roll';
    }
    if (offset.type === 'time' && offset.seconds >= this.config.contentDuration) {
      return 'post-roll';
    }
    return 'mid-roll';
  }

  /**
   * Get position type from raw time
   */
  private getPositionFromTime(time: number): 'pre-roll' | 'mid-roll' | 'post-roll' {
    if (time === 0) {
      return 'pre-roll';
    }
    if (time >= this.config.contentDuration) {
      return 'post-roll';
    }
    return 'mid-roll';
  }

  /**
   * Create ad pod from VAST document
   */
  private createPodFromVAST(
    vastDoc: VASTDocument,
    adBreak: VMAPAdBreak | undefined,
    startTime: number,
    position: 'pre-roll' | 'mid-roll' | 'post-roll',
    warnings: SchedulerWarning[],
    maxDuration?: number
  ): ScheduledAdPod | null {
    const effectiveMaxDuration = maxDuration ?? this.config.maxPodDuration;
    const ads: ScheduledAd[] = [];
    let podOffset = 0;

    // Sort ads by sequence
    const sortedAds = [...vastDoc.ads].sort((a, b) => (a.sequence ?? 0) - (b.sequence ?? 0));

    for (const vastAd of sortedAds) {
      // Check pod limits
      if (ads.length >= this.config.maxAdsPerPod) {
        warnings.push({
          code: 'MAX_ADS_REACHED',
          message: `Pod reached maximum ads (${this.config.maxAdsPerPod})`,
        });
        break;
      }

      // Extract creative
      const creative = extractSSAICreative(vastAd, this.config.mediaPreferences);

      if (!creative) {
        warnings.push({
          code: 'NO_CREATIVE',
          message: `Could not extract creative from ad ${vastAd.id}`,
          context: { adId: vastAd.id },
        });
        continue;
      }

      // Validate duration
      if (creative.duration < this.config.minAdDuration) {
        warnings.push({
          code: 'AD_TOO_SHORT',
          message: `Ad ${vastAd.id} duration (${creative.duration}s) below minimum (${this.config.minAdDuration}s)`,
          context: { adId: vastAd.id, duration: creative.duration },
        });
        continue;
      }

      if (creative.duration > this.config.maxAdDuration) {
        warnings.push({
          code: 'AD_TOO_LONG',
          message: `Ad ${vastAd.id} duration (${creative.duration}s) exceeds maximum (${this.config.maxAdDuration}s)`,
          context: { adId: vastAd.id, duration: creative.duration },
        });
        continue;
      }

      // Check if adding this ad exceeds pod duration
      if (podOffset + creative.duration > effectiveMaxDuration) {
        warnings.push({
          code: 'POD_DURATION_EXCEEDED',
          message: `Skipping ad ${vastAd.id}, would exceed pod duration`,
          context: {
            adId: vastAd.id,
            currentOffset: podOffset,
            adDuration: creative.duration,
            maxDuration: effectiveMaxDuration,
          },
        });
        break;
      }

      // Handle skip offset
      if (!this.config.allowSkippable && creative.skipOffsetSeconds !== null) {
        // Remove skip capability if not allowed
        creative.skipOffsetSeconds = null;
      }

      ads.push({
        id: `${vastAd.id}_${ads.length}`,
        sequence: ads.length + 1,
        duration: creative.duration,
        creative,
        podOffset,
        sourceAd: vastAd,
      });

      podOffset += creative.duration;
    }

    if (ads.length === 0) {
      return null;
    }

    // Gather tracking URLs from VMAP break
    const tracking = {
      breakStart: [] as string[],
      breakEnd: [] as string[],
      error: [] as string[],
    };

    if (adBreak) {
      for (const te of adBreak.trackingEvents) {
        if (te.event === 'breakStart') {
          tracking.breakStart.push(te.url);
        } else if (te.event === 'breakEnd') {
          tracking.breakEnd.push(te.url);
        } else if (te.event === 'error') {
          tracking.error.push(te.url);
        }
      }
    }

    return {
      id: `pod_${startTime}_${Date.now()}`,
      position,
      startTime,
      duration: podOffset,
      ads,
      breakId: adBreak?.breakId,
      repeatInterval: adBreak?.repeatAfter,
      tracking,
    };
  }

  /**
   * Expand pods that have repeat intervals
   */
  private expandRepeatIntervals(
    pods: ScheduledAdPod[],
    warnings: SchedulerWarning[]
  ): ScheduledAdPod[] {
    const expanded: ScheduledAdPod[] = [];

    for (const pod of pods) {
      expanded.push(pod);

      // Handle repeat intervals (only for mid-rolls typically)
      if (pod.repeatInterval && pod.position === 'mid-roll') {
        let repeatCount = 0;
        const maxRepeats = 100; // Safety limit

        while (repeatCount < maxRepeats) {
          // Use multiplication to avoid floating-point drift accumulation
          const nextStart = roundToMs(pod.startTime + pod.repeatInterval * (repeatCount + 1));
          
          if (nextStart >= this.config.contentDuration - pod.duration) {
            break;
          }
          
          // Clone the pod with new timing
          const repeatPod: ScheduledAdPod = {
            ...pod,
            id: `${pod.id}_repeat_${repeatCount + 1}`,
            startTime: nextStart,
            repeatInterval: undefined, // Don't repeat the repeat
          };

          expanded.push(repeatPod);

          repeatCount++;
        }

        if (repeatCount >= maxRepeats) {
          warnings.push({
            code: 'MAX_REPEATS_REACHED',
            message: `Pod ${pod.id} reached maximum repeat count`,
          });
        }
      }
    }

    return expanded;
  }

  /**
   * Enforce minimum gap between adjacent mid-roll pods
   */
  private enforceMinimumGap(
    pods: ScheduledAdPod[],
    warnings: SchedulerWarning[]
  ): ScheduledAdPod[] {
    const result: ScheduledAdPod[] = [];
    let lastMidRollEnd = -Infinity;

    for (const pod of pods) {
      if (pod.position === 'mid-roll') {
        const gap = roundToMs(pod.startTime - lastMidRollEnd);

        if (gap < this.config.minPodGap && lastMidRollEnd >= 0) {
          // Shift this pod forward
          const newStart = roundToMs(lastMidRollEnd + this.config.minPodGap);

          if (newStart + pod.duration < this.config.contentDuration) {
            warnings.push({
              code: 'POD_SHIFTED',
              message: `Mid-roll pod shifted from ${pod.startTime}s to ${newStart}s to maintain minimum gap`,
              context: { originalStart: pod.startTime, newStart },
            });

            result.push({ ...pod, startTime: newStart });
            lastMidRollEnd = roundToMs(newStart + pod.duration);
          } else {
            warnings.push({
              code: 'POD_DROPPED',
              message: `Mid-roll pod at ${pod.startTime}s dropped due to insufficient gap`,
              context: { startTime: pod.startTime },
            });
          }
        } else {
          result.push(pod);
          lastMidRollEnd = roundToMs(pod.startTime + pod.duration);
        }
      } else {
        // Pre-roll and post-roll are always included
        result.push(pod);
      }
    }

    return result;
  }
}

// =============================================================================
// Utility Functions
// =============================================================================

/**
 * Calculate total ad load (percentage of content that is ads)
 */
export function calculateAdLoad(schedule: ScheduleResult, contentDuration: number): number {
  if (contentDuration <= 0) {
    return 0;
  }
  return (schedule.totalAdTime / contentDuration) * 100;
}

/**
 * Get pods at a specific time (for seeking)
 * Uses inclusive start and exclusive end: [startTime, endTime)
 */
export function getPodsAtTime(
  pods: ScheduledAdPod[],
  time: number
): ScheduledAdPod | undefined {
  return pods.find(
    (pod) =>
      time >= pod.startTime &&
      time < roundToMs(pod.startTime + pod.duration)
  );
}

/**
 * Get next pod after a specific time
 */
export function getNextPod(
  pods: ScheduledAdPod[],
  time: number
): ScheduledAdPod | undefined {
  return pods.find((pod) => pod.startTime > time);
}

/**
 * Calculate content time from absolute time (accounting for ads)
 */
export function absoluteToContentTime(
  absoluteTime: number,
  pods: ScheduledAdPod[]
): number {
  let adTimeElapsed = 0;

  for (const pod of pods) {
    const podEndTime = roundToMs(pod.startTime + pod.duration);
    if (absoluteTime >= podEndTime) {
      // Completely past this pod
      adTimeElapsed += pod.duration;
    } else if (absoluteTime >= pod.startTime) {
      // Currently in this pod
      adTimeElapsed += absoluteTime - pod.startTime;
    }
  }

  return roundToMs(absoluteTime - adTimeElapsed);
}

/**
 * Calculate absolute time from content time (accounting for ads)
 */
export function contentToAbsoluteTime(
  contentTime: number,
  pods: ScheduledAdPod[]
): number {
  let absoluteTime = contentTime;

  for (const pod of pods) {
    if (absoluteTime >= pod.startTime) {
      absoluteTime = roundToMs(absoluteTime + pod.duration);
    }
  }

  return absoluteTime;
}

/**
 * Generate a timeline representation
 */
export function generateTimeline(
  schedule: ScheduleResult,
  contentDuration: number
): Array<{
  type: 'content' | 'ad-pod';
  startTime: number;
  endTime: number;
  duration: number;
  podId?: string;
}> {
  const timeline: Array<{
    type: 'content' | 'ad-pod';
    startTime: number;
    endTime: number;
    duration: number;
    podId?: string;
  }> = [];

  let currentTime = 0;

  for (const pod of schedule.pods) {
    // Content before this pod
    if (pod.startTime > currentTime) {
      const contentDur = roundToMs(pod.startTime - currentTime);
      timeline.push({
        type: 'content',
        startTime: currentTime,
        endTime: pod.startTime,
        duration: contentDur,
      });
    }

    // The pod itself
    const podEndTime = roundToMs(pod.startTime + pod.duration);
    timeline.push({
      type: 'ad-pod',
      startTime: pod.startTime,
      endTime: podEndTime,
      duration: pod.duration,
      podId: pod.id,
    });

    currentTime = podEndTime;
  }

  // Remaining content after last pod
  if (currentTime < contentDuration) {
    const remainingDur = roundToMs(contentDuration - currentTime);
    timeline.push({
      type: 'content',
      startTime: currentTime,
      endTime: contentDuration,
      duration: remainingDur,
    });
  }

  return timeline;
}

// =============================================================================
// Exports
// =============================================================================

export default AdPodScheduler;
