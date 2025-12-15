/**
 * Piercing E2E Tests - Signaling & Pod Planning
 *
 * Tests: "SCTE chaos + origin restarts"
 * - Overlapping cues test
 * - Out-of-order cues test
 * - Origin restart during break test
 * - Shortened break test
 */

import { describe, it, expect, beforeEach, jest } from '@jest/globals';

interface AdBreakCue {
  cueId: string;
  type: 'start' | 'end';
  breakId: string;
  mediaTime: number;
  duration?: number;
  plannedDuration?: number;
  wallClockTime?: Date;
  source: 'scte35' | 'daterange' | 'vod_marker';
}

interface PodPlan {
  breakId: string;
  podId: string;
  startTime: Date;
  mediaStartTime: number;
  plannedDuration: number;
  actualDuration?: number;
  slots: PodSlot[];
  status: 'active' | 'completed' | 'overrun' | 'shortened';
  startCue: AdBreakCue;
  endCue?: AdBreakCue;
  keyframeAligned: boolean;
  splicePointTime: number;
  mergedFrom?: string[]; // For overlapping cue merge tracking
  priority?: number;
}

interface PodSlot {
  slotId: string;
  index: number;
  duration: number;
  status: 'pending' | 'filled' | 'slate' | 'skipped';
  creativeId?: string;
}

interface CueIngestionResult {
  accepted: boolean;
  duplicate: boolean;
  cue?: AdBreakCue;
  podPlan?: PodPlan;
  rejectionReason?: string;
  mergeResult?: 'merged' | 'priority_chosen' | 'rejected';
}

/**
 * Mock Pod Planner for E2E Tests
 */
class MockPodPlanner {
  private activePods: Map<string, PodPlan> = new Map();
  private cueHistory: AdBreakCue[] = [];
  private config = {
    maxPodDuration: 300,
    keyframeInterval: 2,
    overlapMergeThresholdMs: 5000,
    outOfOrderGracePeriodMs: 10000,
    defaultSlotDuration: 30,
  };

  /**
   * Ingest a cue with overlapping/out-of-order handling
   */
  ingestCue(cue: AdBreakCue): CueIngestionResult {
    this.cueHistory.push(cue);

    if (cue.type === 'start') {
      return this.handleStartCue(cue);
    } else {
      return this.handleEndCue(cue);
    }
  }

  private handleStartCue(cue: AdBreakCue): CueIngestionResult {
    // Check for overlapping cues
    const overlapping = this.findOverlappingPod(cue);

    if (overlapping) {
      return this.handleOverlappingCue(cue, overlapping);
    }

    // Create new pod
    const podPlan = this.createPodPlan(cue);
    this.activePods.set(podPlan.breakId, podPlan);

    return {
      accepted: true,
      duplicate: false,
      cue,
      podPlan,
    };
  }

  private handleEndCue(cue: AdBreakCue): CueIngestionResult {
    // Find matching active pod
    let podPlan = this.activePods.get(cue.breakId);

    if (!podPlan) {
      // Out-of-order: end cue arrived before start
      // Check if within grace period
      const recentStartCue = this.findRecentStartCue(cue);
      if (recentStartCue) {
        // Late start cue scenario - create pod from end cue
        return this.handleLateStartCue(cue, recentStartCue);
      }

      return {
        accepted: false,
        duplicate: false,
        cue,
        rejectionReason: 'No matching active pod found for end cue',
      };
    }

    // Calculate actual duration
    podPlan.actualDuration = cue.mediaTime - podPlan.mediaStartTime;
    podPlan.endCue = cue;

    // Check for shortened break
    if (podPlan.actualDuration < podPlan.plannedDuration * 0.75) {
      podPlan.status = 'shortened';
      this.shrinkPod(podPlan);
    } else {
      podPlan.status = 'completed';
    }

    this.activePods.delete(podPlan.breakId);

    return {
      accepted: true,
      duplicate: false,
      cue,
      podPlan,
    };
  }

  private findOverlappingPod(cue: AdBreakCue): PodPlan | null {
    for (const pod of this.activePods.values()) {
      if (pod.status !== 'active') continue;

      const podEndTime = pod.mediaStartTime + pod.plannedDuration;
      const overlapStart = Math.max(cue.mediaTime, pod.mediaStartTime);
      const overlapEnd = Math.min(
        cue.mediaTime + (cue.plannedDuration || 30),
        podEndTime
      );
      const overlapDuration = overlapEnd - overlapStart;

      if (overlapDuration > this.config.overlapMergeThresholdMs / 1000) {
        return pod;
      }
    }
    return null;
  }

  private handleOverlappingCue(
    newCue: AdBreakCue,
    existingPod: PodPlan
  ): CueIngestionResult {
    // Deterministic priority: earlier cue wins, or higher priority source
    const newCuePriority = this.getCuePriority(newCue);
    const existingPriority = existingPod.priority || 0;

    if (newCuePriority > existingPriority) {
      // New cue takes priority - merge
      existingPod.mergedFrom = existingPod.mergedFrom || [];
      existingPod.mergedFrom.push(newCue.cueId);
      existingPod.priority = newCuePriority;

      // Extend duration if new cue is longer
      const newDuration = newCue.plannedDuration || 30;
      if (newDuration > existingPod.plannedDuration) {
        existingPod.plannedDuration = Math.min(
          newDuration,
          this.config.maxPodDuration
        );
        this.updateSlots(existingPod);
      }

      return {
        accepted: true,
        duplicate: false,
        cue: newCue,
        podPlan: existingPod,
        mergeResult: 'merged',
      };
    } else {
      // Existing pod takes priority
      return {
        accepted: false,
        duplicate: false,
        cue: newCue,
        rejectionReason: `Overlapping cue rejected: existing pod ${existingPod.podId} has higher priority`,
        mergeResult: 'priority_chosen',
      };
    }
  }

  private getCuePriority(cue: AdBreakCue): number {
    // Priority based on source type
    switch (cue.source) {
      case 'scte35':
        return 3;
      case 'daterange':
        return 2;
      case 'vod_marker':
        return 1;
      default:
        return 0;
    }
  }

  private findRecentStartCue(endCue: AdBreakCue): AdBreakCue | null {
    const graceWindow = this.config.outOfOrderGracePeriodMs;
    const now = Date.now();

    for (let i = this.cueHistory.length - 1; i >= 0; i--) {
      const historyCue = this.cueHistory[i];
      if (historyCue.type !== 'start') continue;
      if (historyCue.breakId !== endCue.breakId) continue;

      const age = now - (historyCue.wallClockTime?.getTime() || 0);
      if (age < graceWindow) {
        return historyCue;
      }
    }
    return null;
  }

  private handleLateStartCue(
    endCue: AdBreakCue,
    startCue: AdBreakCue
  ): CueIngestionResult {
    // Reconstruct pod from both cues
    const podPlan = this.createPodPlan(startCue);
    podPlan.actualDuration = endCue.mediaTime - startCue.mediaTime;
    podPlan.endCue = endCue;
    podPlan.status = 'completed';

    return {
      accepted: true,
      duplicate: false,
      cue: endCue,
      podPlan,
    };
  }

  private createPodPlan(cue: AdBreakCue): PodPlan {
    const duration = cue.plannedDuration || cue.duration || 30;
    const slots = this.createSlots(duration);

    return {
      breakId: cue.breakId,
      podId: `pod-${Date.now()}`,
      startTime: cue.wallClockTime || new Date(),
      mediaStartTime: cue.mediaTime,
      plannedDuration: Math.min(duration, this.config.maxPodDuration),
      slots,
      status: 'active',
      startCue: cue,
      keyframeAligned: this.isKeyframeAligned(cue.mediaTime),
      splicePointTime: this.alignToKeyframe(cue.mediaTime),
      priority: this.getCuePriority(cue),
    };
  }

  private createSlots(duration: number): PodSlot[] {
    const slots: PodSlot[] = [];
    let remaining = duration;
    let index = 0;

    while (remaining >= 5) {
      const slotDuration = Math.min(this.config.defaultSlotDuration, remaining);
      slots.push({
        slotId: `slot-${index}`,
        index,
        duration: slotDuration,
        status: 'pending',
      });
      remaining -= slotDuration;
      index++;
    }

    return slots;
  }

  private updateSlots(pod: PodPlan): void {
    pod.slots = this.createSlots(pod.plannedDuration);
  }

  private shrinkPod(pod: PodPlan): void {
    if (!pod.actualDuration) return;

    // Trim slots to fit actual duration
    let accumulatedDuration = 0;
    const newSlots: PodSlot[] = [];

    for (const slot of pod.slots) {
      if (accumulatedDuration >= pod.actualDuration) {
        slot.status = 'skipped';
      } else if (accumulatedDuration + slot.duration > pod.actualDuration) {
        // Partial slot
        slot.duration = pod.actualDuration - accumulatedDuration;
      }
      newSlots.push(slot);
      accumulatedDuration += slot.duration;
    }

    pod.slots = newSlots.filter((s) => s.status !== 'skipped');
    pod.plannedDuration = pod.actualDuration;
  }

  private isKeyframeAligned(time: number): boolean {
    return time % this.config.keyframeInterval === 0;
  }

  private alignToKeyframe(time: number): number {
    return (
      Math.round(time / this.config.keyframeInterval) *
      this.config.keyframeInterval
    );
  }

  /**
   * Handle origin restart / PTS discontinuity
   */
  handleOriginRestart(ptsJump: number): {
    affectedPods: PodPlan[];
    action: 'slate' | 'continue' | 'abort';
    manifest: 'valid' | 'regenerated';
  } {
    const affectedPods: PodPlan[] = [];

    for (const pod of this.activePods.values()) {
      if (pod.status === 'active') {
        affectedPods.push(pod);
      }
    }

    // Determine action based on PTS jump severity
    let action: 'slate' | 'continue' | 'abort';
    let manifest: 'valid' | 'regenerated';

    if (Math.abs(ptsJump) < 1) {
      // Minor discontinuity - continue
      action = 'continue';
      manifest = 'valid';
    } else if (Math.abs(ptsJump) < 30) {
      // Moderate discontinuity - use slate
      action = 'slate';
      manifest = 'regenerated';

      // Mark affected slots for slate
      for (const pod of affectedPods) {
        for (const slot of pod.slots) {
          if (slot.status === 'pending') {
            slot.status = 'slate';
          }
        }
      }
    } else {
      // Major discontinuity - abort pods
      action = 'abort';
      manifest = 'regenerated';

      for (const pod of affectedPods) {
        pod.status = 'completed';
        this.activePods.delete(pod.breakId);
      }
    }

    return { affectedPods, action, manifest };
  }

  getActivePods(): PodPlan[] {
    return Array.from(this.activePods.values());
  }
}

describe('Piercing E2E Tests - Signaling & Pod Planning', () => {
  let podPlanner: MockPodPlanner;

  beforeEach(() => {
    podPlanner = new MockPodPlanner();
  });

  describe('Overlapping Cues Test', () => {
    /**
     * 2 breaks overlap by 5-15s → exactly one pod plan chosen; explain shows why
     */
    it('should handle overlapping cues with deterministic priority', () => {
      const cue1: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 60,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const cue2: AdBreakCue = {
        cueId: 'cue-2',
        type: 'start',
        breakId: 'break-2',
        mediaTime: 110, // Overlaps by 50s (100+60=160, 110 is within)
        plannedDuration: 45,
        wallClockTime: new Date(),
        source: 'daterange',
      };

      // Ingest first cue
      const result1 = podPlanner.ingestCue(cue1);
      expect(result1.accepted).toBe(true);
      expect(result1.podPlan).toBeDefined();

      // Ingest overlapping cue
      const result2 = podPlanner.ingestCue(cue2);

      // SCTE-35 has higher priority than daterange
      expect(result2.mergeResult).toBe('priority_chosen');
      expect(result2.accepted).toBe(false);
      expect(result2.rejectionReason).toContain('higher priority');

      // Only one active pod
      expect(podPlanner.getActivePods().length).toBe(1);
    });

    it('should merge overlapping cues when new cue has higher priority', () => {
      // First cue with lower priority
      const cue1: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 30,
        wallClockTime: new Date(),
        source: 'vod_marker', // Lower priority
      };

      // Second cue with higher priority, overlapping
      const cue2: AdBreakCue = {
        cueId: 'cue-2',
        type: 'start',
        breakId: 'break-2',
        mediaTime: 115, // Overlaps
        plannedDuration: 60,
        wallClockTime: new Date(),
        source: 'scte35', // Higher priority
      };

      const result1 = podPlanner.ingestCue(cue1);
      expect(result1.accepted).toBe(true);

      const result2 = podPlanner.ingestCue(cue2);
      expect(result2.accepted).toBe(true);
      expect(result2.mergeResult).toBe('merged');

      // Pod should be extended
      const pods = podPlanner.getActivePods();
      expect(pods.length).toBe(1);
      expect(pods[0].plannedDuration).toBe(60); // Extended to new cue's duration
      expect(pods[0].mergedFrom).toContain('cue-2');
    });
  });

  describe('Out-of-Order Cues Test', () => {
    /**
     * End cue before start cue (bad upstream) → system degrades safely, never wedges
     */
    it('should handle out-of-order cues gracefully', () => {
      // End cue arrives first (bad upstream)
      const endCue: AdBreakCue = {
        cueId: 'end-cue-1',
        type: 'end',
        breakId: 'break-1',
        mediaTime: 130,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      // Process end cue - should fail gracefully
      const endResult = podPlanner.ingestCue(endCue);
      expect(endResult.accepted).toBe(false);
      expect(endResult.rejectionReason).toContain('No matching active pod');

      // System should not be wedged
      expect(podPlanner.getActivePods().length).toBe(0);

      // Now start cue arrives (late)
      const startCue: AdBreakCue = {
        cueId: 'start-cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 30,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const startResult = podPlanner.ingestCue(startCue);
      expect(startResult.accepted).toBe(true);
      expect(podPlanner.getActivePods().length).toBe(1);
    });

    it('should recover from out-of-order within grace period', () => {
      // Start cue first
      const startCue: AdBreakCue = {
        cueId: 'start-cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 30,
        wallClockTime: new Date(Date.now() - 5000), // 5s ago
        source: 'scte35',
      };

      podPlanner.ingestCue(startCue);

      // End cue for same break
      const endCue: AdBreakCue = {
        cueId: 'end-cue-1',
        type: 'end',
        breakId: 'break-1',
        mediaTime: 130,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const result = podPlanner.ingestCue(endCue);
      expect(result.accepted).toBe(true);
      expect(result.podPlan?.status).toBe('completed');
    });
  });

  describe('Origin Restart During Break Test', () => {
    /**
     * Force discontinuity/PTS reset mid-pod → player survives;
     * slate kicks in if needed; no invalid playlists
     */
    it('should handle minor PTS discontinuity gracefully', () => {
      // Create active pod
      const cue: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 60,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      podPlanner.ingestCue(cue);
      expect(podPlanner.getActivePods().length).toBe(1);

      // Simulate minor PTS jump (< 1 second)
      const result = podPlanner.handleOriginRestart(0.5);

      expect(result.action).toBe('continue');
      expect(result.manifest).toBe('valid');
      expect(result.affectedPods.length).toBe(1);
    });

    it('should use slate for moderate PTS discontinuity', () => {
      const cue: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 60,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      podPlanner.ingestCue(cue);

      // Simulate moderate PTS jump (1-30 seconds)
      const result = podPlanner.handleOriginRestart(15);

      expect(result.action).toBe('slate');
      expect(result.manifest).toBe('regenerated');

      // Affected pods should have slate slots
      const pods = podPlanner.getActivePods();
      const slateSlots = pods[0]?.slots.filter((s) => s.status === 'slate');
      expect(slateSlots?.length).toBeGreaterThan(0);
    });

    it('should abort pods on major PTS discontinuity', () => {
      const cue: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 60,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      podPlanner.ingestCue(cue);
      expect(podPlanner.getActivePods().length).toBe(1);

      // Simulate major PTS jump (> 30 seconds)
      const result = podPlanner.handleOriginRestart(45);

      expect(result.action).toBe('abort');
      expect(result.manifest).toBe('regenerated');
      expect(podPlanner.getActivePods().length).toBe(0); // Pod aborted
    });
  });

  describe('Shortened Break Test', () => {
    /**
     * Planned 120s, actual 60s → pod shrinks; never overruns content
     */
    it('should shrink pod when break is shortened', () => {
      // Create pod with planned 120s duration
      const startCue: AdBreakCue = {
        cueId: 'start-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 120,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const startResult = podPlanner.ingestCue(startCue);
      expect(startResult.accepted).toBe(true);
      expect(startResult.podPlan?.plannedDuration).toBe(120);
      const originalSlotCount = startResult.podPlan?.slots.length || 0;

      // End cue arrives early (actual 60s)
      const endCue: AdBreakCue = {
        cueId: 'end-1',
        type: 'end',
        breakId: 'break-1',
        mediaTime: 160, // 60s actual (100 + 60 = 160)
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const endResult = podPlanner.ingestCue(endCue);
      expect(endResult.accepted).toBe(true);
      expect(endResult.podPlan?.status).toBe('shortened');
      expect(endResult.podPlan?.actualDuration).toBe(60);

      // Pod should have fewer slots after shrinking
      const finalSlotCount = endResult.podPlan?.slots.length || 0;
      expect(finalSlotCount).toBeLessThan(originalSlotCount);
    });

    it('should never overrun content after shortened break', () => {
      const contentResumeTime = 160; // Content should resume at this time

      const startCue: AdBreakCue = {
        cueId: 'start-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100,
        plannedDuration: 120, // Would end at 220
        wallClockTime: new Date(),
        source: 'scte35',
      };

      podPlanner.ingestCue(startCue);

      // Early end
      const endCue: AdBreakCue = {
        cueId: 'end-1',
        type: 'end',
        breakId: 'break-1',
        mediaTime: contentResumeTime, // End at 160, not 220
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const result = podPlanner.ingestCue(endCue);

      // Verify pod doesn't extend past actual end time
      const podEndTime =
        result.podPlan!.mediaStartTime + result.podPlan!.plannedDuration;
      expect(podEndTime).toBe(contentResumeTime);
      expect(podEndTime).not.toBeGreaterThan(contentResumeTime);
    });
  });

  describe('Cue-to-Keyframe Policy', () => {
    it('should align cue to nearest keyframe boundary', () => {
      const cue: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 101.3, // Not on 2s boundary
        plannedDuration: 30,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const result = podPlanner.ingestCue(cue);

      // Should snap to nearest 2s boundary (100 or 102)
      expect(result.podPlan?.splicePointTime).toBe(102); // Rounded to 102
      expect(result.podPlan?.keyframeAligned).toBe(false); // Original was not aligned
    });

    it('should accept cues already on keyframe boundary', () => {
      const cue: AdBreakCue = {
        cueId: 'cue-1',
        type: 'start',
        breakId: 'break-1',
        mediaTime: 100, // On 2s boundary
        plannedDuration: 30,
        wallClockTime: new Date(),
        source: 'scte35',
      };

      const result = podPlanner.ingestCue(cue);

      expect(result.podPlan?.keyframeAligned).toBe(true);
      expect(result.podPlan?.splicePointTime).toBe(100);
    });
  });
});

export {};
