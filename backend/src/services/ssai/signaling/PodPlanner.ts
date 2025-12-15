/**
 * Pod Planner
 *
 * Normalizes ad break cues into pod plans with slot allocation.
 */

import crypto from 'crypto';
import {
  AdBreakCue,
  PodPlan,
  PodSlot,
  CueDedupeKey,
  DriftDetection,
  DriftBehavior,
  CueIngestionResult,
} from './types';

/**
 * Pod planner configuration
 */
export interface PodPlannerConfig {
  /** Default slot duration in seconds */
  defaultSlotDuration: number;

  /** Minimum slot duration in seconds */
  minSlotDuration: number;

  /** Maximum pod duration in seconds */
  maxPodDuration: number;

  /** Keyframe interval in seconds (for alignment) */
  keyframeInterval: number;

  /** Drift tolerance in seconds */
  driftToleranceSeconds: number;

  /** Cue deduplication window in seconds */
  dedupeWindowSeconds: number;
}

/**
 * Default configuration
 */
const DEFAULT_CONFIG: PodPlannerConfig = {
  defaultSlotDuration: 30,
  minSlotDuration: 5,
  maxPodDuration: 300, // 5 minutes max
  keyframeInterval: 2,
  driftToleranceSeconds: 1,
  dedupeWindowSeconds: 5,
};

/**
 * Pod Planner
 */
export class PodPlanner {
  private config: PodPlannerConfig;
  private activePods: Map<string, PodPlan> = new Map();
  private recentCues: Map<string, number> = new Map(); // For deduplication

  constructor(config: Partial<PodPlannerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };

    // Cleanup old dedup entries periodically
    setInterval(() => this.cleanupDedupeCache(), 60000);
  }

  /**
   * Ingest a cue and create/update pod plan
   */
  ingestCue(cue: AdBreakCue): CueIngestionResult {
    // Check for duplicate
    if (this.isDuplicate(cue)) {
      return {
        accepted: false,
        duplicate: true,
        rejectionReason: 'Duplicate cue detected',
      };
    }

    // Record cue for deduplication
    this.recordCue(cue);

    // Handle based on cue type
    if (cue.type === 'start') {
      return this.handleStartCue(cue);
    } else {
      return this.handleEndCue(cue);
    }
  }

  /**
   * Handle start cue
   */
  private handleStartCue(cue: AdBreakCue): CueIngestionResult {
    // Check if we already have an active pod for this break
    if (cue.breakId && this.activePods.has(cue.breakId)) {
      return {
        accepted: false,
        duplicate: true,
        rejectionReason: 'Pod already exists for this break',
      };
    }

    // Create pod plan
    const podPlan = this.createPodPlan(cue);

    // Store active pod
    this.activePods.set(podPlan.breakId, podPlan);

    return {
      accepted: true,
      cue,
      duplicate: false,
      podPlan,
    };
  }

  /**
   * Handle end cue
   */
  private handleEndCue(cue: AdBreakCue): CueIngestionResult {
    // Find matching active pod
    let podPlan: PodPlan | undefined;

    if (cue.breakId) {
      podPlan = this.activePods.get(cue.breakId);
    }

    // If no explicit match, find by time proximity
    if (!podPlan) {
      for (const [, pod] of this.activePods) {
        if (pod.status === 'active') {
          const podEndTime = pod.mediaStartTime + pod.plannedDuration;
          if (Math.abs(cue.mediaTime - podEndTime) < this.config.driftToleranceSeconds * 2) {
            podPlan = pod;
            break;
          }
        }
      }
    }

    if (!podPlan) {
      return {
        accepted: false,
        cue,
        duplicate: false,
        rejectionReason: 'No matching active pod found',
      };
    }

    // Update pod with end cue
    podPlan.endCue = cue;
    podPlan.actualDuration = cue.mediaTime - podPlan.mediaStartTime;
    podPlan.status = 'completed';

    // Remove from active pods
    this.activePods.delete(podPlan.breakId);

    return {
      accepted: true,
      cue,
      duplicate: false,
      podPlan,
    };
  }

  /**
   * Create a pod plan from a start cue
   */
  private createPodPlan(cue: AdBreakCue): PodPlan {
    const breakId = cue.breakId || `break_${crypto.randomUUID()}`;
    const podId = `pod_${crypto.randomUUID()}`;

    // Determine duration
    const duration = cue.duration || cue.plannedDuration || this.config.defaultSlotDuration;
    const clampedDuration = Math.min(duration, this.config.maxPodDuration);

    // Calculate aligned splice point
    const splicePointTime = this.alignToKeyframe(cue.mediaTime);
    const keyframeAligned = splicePointTime === cue.mediaTime;

    // Create slots
    const slots = this.createSlots(clampedDuration);

    return {
      breakId,
      podId,
      startTime: cue.wallClockTime || new Date(),
      mediaStartTime: cue.mediaTime,
      plannedDuration: clampedDuration,
      slots,
      status: 'active',
      startCue: cue,
      keyframeAligned,
      splicePointTime,
    };
  }

  /**
   * Create slots for a pod
   */
  private createSlots(totalDuration: number): PodSlot[] {
    const slots: PodSlot[] = [];
    let remainingDuration = totalDuration;
    let index = 0;

    // Create slots until duration is filled
    while (remainingDuration >= this.config.minSlotDuration) {
      const slotDuration = Math.min(
        this.config.defaultSlotDuration,
        remainingDuration
      );

      slots.push({
        slotId: `slot_${crypto.randomUUID()}`,
        index,
        duration: slotDuration,
        status: 'pending',
      });

      remainingDuration -= slotDuration;
      index++;
    }

    return slots;
  }

  /**
   * Align time to nearest keyframe boundary
   */
  private alignToKeyframe(time: number): number {
    const interval = this.config.keyframeInterval;
    return Math.round(time / interval) * interval;
  }

  /**
   * Check if cue is a duplicate
   */
  private isDuplicate(cue: AdBreakCue): boolean {
    const key = this.getDedupeKey(cue);
    const existing = this.recentCues.get(key);

    if (!existing) {
      return false;
    }

    const now = Date.now();
    const windowMs = this.config.dedupeWindowSeconds * 1000;

    return now - existing < windowMs;
  }

  /**
   * Record cue for deduplication
   */
  private recordCue(cue: AdBreakCue): void {
    const key = this.getDedupeKey(cue);
    this.recentCues.set(key, Date.now());
  }

  /**
   * Generate deduplication key
   */
  private getDedupeKey(cue: AdBreakCue): string {
    const timeWindow = Math.floor(cue.mediaTime / this.config.dedupeWindowSeconds);
    return `${cue.cueId || cue.breakId || ''}:${timeWindow}:${cue.source}`;
  }

  /**
   * Cleanup old dedup entries
   */
  private cleanupDedupeCache(): void {
    const now = Date.now();
    const maxAge = this.config.dedupeWindowSeconds * 1000 * 2;

    for (const [key, timestamp] of this.recentCues) {
      if (now - timestamp > maxAge) {
        this.recentCues.delete(key);
      }
    }
  }

  /**
   * Detect drift between expected and actual cue timing
   */
  detectDrift(expectedTime: number, actualTime: number): DriftDetection {
    const drift = actualTime - expectedTime;
    const absoluteDrift = Math.abs(drift);

    let severity: 'none' | 'minor' | 'major' | 'critical';
    let recommendedBehavior: DriftBehavior;

    if (absoluteDrift < 0.1) {
      severity = 'none';
      recommendedBehavior = 'partial_fill';
    } else if (absoluteDrift < this.config.driftToleranceSeconds) {
      severity = 'minor';
      recommendedBehavior = 'partial_fill';
    } else if (absoluteDrift < this.config.driftToleranceSeconds * 3) {
      severity = 'major';
      recommendedBehavior = drift > 0 ? 'slate' : 'skip';
    } else {
      severity = 'critical';
      recommendedBehavior = 'slate';
    }

    return {
      driftSeconds: drift,
      absoluteDrift,
      severity,
      recommendedBehavior,
    };
  }

  /**
   * Get active pod by break ID
   */
  getActivePod(breakId: string): PodPlan | undefined {
    return this.activePods.get(breakId);
  }

  /**
   * Get all active pods
   */
  getAllActivePods(): PodPlan[] {
    return Array.from(this.activePods.values());
  }

  /**
   * Update slot status
   */
  updateSlotStatus(
    podId: string,
    slotId: string,
    status: PodSlot['status'],
    creativeId?: string
  ): boolean {
    for (const pod of this.activePods.values()) {
      if (pod.podId === podId) {
        const slot = pod.slots.find((s) => s.slotId === slotId);
        if (slot) {
          slot.status = status;
          if (creativeId) {
            slot.creativeId = creativeId;
          }
          return true;
        }
      }
    }
    return false;
  }

  /**
   * Calculate fill rate for a pod
   */
  calculateFillRate(pod: PodPlan): number {
    if (pod.slots.length === 0) {
      return 0;
    }

    const filledSlots = pod.slots.filter(
      (s) => s.status === 'filled'
    ).length;

    return filledSlots / pod.slots.length;
  }

  /**
   * Get total filled duration for a pod
   */
  getFilledDuration(pod: PodPlan): number {
    return pod.slots
      .filter((s) => s.status === 'filled')
      .reduce((sum, s) => sum + s.duration, 0);
  }

  /**
   * Get unfilled duration for a pod
   */
  getUnfilledDuration(pod: PodPlan): number {
    return pod.slots
      .filter((s) => s.status === 'pending' || s.status === 'slate')
      .reduce((sum, s) => sum + s.duration, 0);
  }
}

// Export singleton instance
export const podPlanner = new PodPlanner();
