/**
 * SSAI VOD Reconciler
 *
 * Handles deterministic quartile calculation for VOD content.
 * Since VOD playback is seekable, we need to handle out-of-order
 * and skipped quartile events.
 */

import {
  TrackingEvent,
  TrackingEventType,
  ReconciliationResult,
  SlotReconciliation,
  Discrepancy,
  QuartilePosition,
} from './types';

interface VODPodInfo {
  podId: string;
  slots: VODSlotInfo[];
  startedAt: Date;
}

interface VODSlotInfo {
  slotId: string;
  creativeId: string;
  duration: number;
  startOffset: number; // Offset within the pod
}

interface PlaybackState {
  currentTime: number;
  duration: number;
  isPlaying: boolean;
  isSeeking: boolean;
  playbackRate: number;
}

interface QuartileState {
  fired: Set<TrackingEventType>;
  positions: QuartilePosition[];
  lastPosition: number;
}

export interface VODReconcilerConfig {
  /** Tolerance for quartile timing (seconds) */
  timingToleranceSeconds: number;

  /** Whether to fire missed quartiles on seek */
  fireOnSeek: boolean;

  /** Maximum age for reconciliation (minutes) */
  maxAgeMinutes: number;
}

const DEFAULT_CONFIG: VODReconcilerConfig = {
  timingToleranceSeconds: 0.5,
  fireOnSeek: true,
  maxAgeMinutes: 120,
};

export class VODReconciler {
  private config: VODReconcilerConfig;
  private pods: Map<string, VODPodInfo> = new Map();
  private quartileStates: Map<string, QuartileState> = new Map();
  private receivedEvents: Map<string, TrackingEvent[]> = new Map();

  constructor(config: Partial<VODReconcilerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Register a VOD pod for reconciliation
   */
  registerPod(podId: string, slots: VODSlotInfo[]): void {
    this.pods.set(podId, {
      podId,
      slots,
      startedAt: new Date(),
    });

    // Initialize quartile states for each slot
    let offset = 0;
    for (const slot of slots) {
      slot.startOffset = offset;
      this.quartileStates.set(slot.slotId, {
        fired: new Set(),
        positions: this.calculateQuartilePositions(slot.duration),
        lastPosition: 0,
      });
      this.receivedEvents.set(slot.slotId, []);
      offset += slot.duration;
    }
  }

  /**
   * Calculate quartile positions for a given duration
   */
  private calculateQuartilePositions(duration: number): QuartilePosition[] {
    return [
      { type: 'firstQuartile', position: duration * 0.25, progress: 25 },
      { type: 'midpoint', position: duration * 0.5, progress: 50 },
      { type: 'thirdQuartile', position: duration * 0.75, progress: 75 },
      { type: 'complete', position: duration, progress: 100 },
    ];
  }

  /**
   * Update playback position and check for quartiles
   */
  updatePosition(
    slotId: string,
    currentTime: number,
    playbackState: PlaybackState
  ): TrackingEventType[] {
    const state = this.quartileStates.get(slotId);
    if (!state) return [];

    const eventsToFire: TrackingEventType[] = [];

    // Handle seeking
    if (playbackState.isSeeking) {
      if (this.config.fireOnSeek) {
        // Fire any missed quartiles between last position and current
        eventsToFire.push(
          ...this.getMissedQuartiles(state, state.lastPosition, currentTime)
        );
      }
    } else {
      // Normal playback - check for quartile crossings
      for (const quartile of state.positions) {
        if (
          !state.fired.has(quartile.type) &&
          this.crossedQuartile(state.lastPosition, currentTime, quartile.position)
        ) {
          eventsToFire.push(quartile.type);
          state.fired.add(quartile.type);
        }
      }
    }

    state.lastPosition = currentTime;
    return eventsToFire;
  }

  /**
   * Check if we crossed a quartile boundary
   */
  private crossedQuartile(
    lastPosition: number,
    currentPosition: number,
    quartilePosition: number
  ): boolean {
    // Normal forward playback
    if (currentPosition > lastPosition) {
      return lastPosition < quartilePosition && currentPosition >= quartilePosition;
    }
    return false;
  }

  /**
   * Get missed quartiles when seeking forward
   */
  private getMissedQuartiles(
    state: QuartileState,
    fromPosition: number,
    toPosition: number
  ): TrackingEventType[] {
    const missed: TrackingEventType[] = [];

    if (toPosition <= fromPosition) return missed;

    for (const quartile of state.positions) {
      if (
        !state.fired.has(quartile.type) &&
        quartile.position > fromPosition &&
        quartile.position <= toPosition
      ) {
        missed.push(quartile.type);
        state.fired.add(quartile.type);
      }
    }

    return missed;
  }

  /**
   * Record a received tracking event
   */
  recordEvent(event: TrackingEvent): void {
    const events = this.receivedEvents.get(event.slotId);
    if (events) {
      events.push(event);
    }
  }

  /**
   * Reconcile a pod
   */
  reconcile(podId: string): ReconciliationResult {
    const pod = this.pods.get(podId);
    if (!pod) {
      return {
        podId,
        slots: [],
        fillRate: 0,
        completionRate: 0,
        discrepancies: [],
      };
    }

    const slotReconciliations: SlotReconciliation[] = [];
    const discrepancies: Discrepancy[] = [];
    let completedSlots = 0;
    let filledSlots = 0;

    for (const slot of pod.slots) {
      const state = this.quartileStates.get(slot.slotId);
      const events = this.receivedEvents.get(slot.slotId) || [];

      // Expected events
      const expectedEvents: TrackingEventType[] = [
        'impression',
        'start',
        'firstQuartile',
        'midpoint',
        'thirdQuartile',
        'complete',
      ];

      // Received events (unique by type)
      const receivedEventTypes = new Set(events.map((e) => e.type));
      const receivedEvents = Array.from(receivedEventTypes);

      // Missing and extra
      const missingEvents = expectedEvents.filter(
        (e) => !receivedEventTypes.has(e)
      );
      const extraEvents = receivedEvents.filter(
        (e) => !expectedEvents.includes(e)
      ) as TrackingEventType[];

      // Check for completion
      const completed = receivedEventTypes.has('complete');
      if (completed) completedSlots++;

      // Check if slot was filled
      if (receivedEventTypes.has('impression') || receivedEventTypes.has('start')) {
        filledSlots++;
      }

      slotReconciliations.push({
        slotId: slot.slotId,
        creativeId: slot.creativeId,
        expectedEvents,
        receivedEvents,
        missingEvents,
        extraEvents,
        completed,
      });

      // Generate discrepancies
      for (const missing of missingEvents) {
        discrepancies.push({
          type: 'missing_event',
          eventType: missing,
          slotId: slot.slotId,
          details: `Expected ${missing} event was not received`,
          severity: missing === 'complete' ? 'warning' : 'info',
        });
      }

      // Check for timing discrepancies
      this.checkTimingDiscrepancies(slot, events, discrepancies);
    }

    return {
      podId,
      slots: slotReconciliations,
      fillRate: pod.slots.length > 0 ? filledSlots / pod.slots.length : 0,
      completionRate: filledSlots > 0 ? completedSlots / filledSlots : 0,
      discrepancies,
    };
  }

  /**
   * Check for timing discrepancies in events
   */
  private checkTimingDiscrepancies(
    slot: VODSlotInfo,
    events: TrackingEvent[],
    discrepancies: Discrepancy[]
  ): void {
    const quartileEvents = events.filter((e) =>
      ['firstQuartile', 'midpoint', 'thirdQuartile', 'complete'].includes(e.type)
    );

    for (const event of quartileEvents) {
      const expectedPosition = this.getExpectedPosition(event.type, slot.duration);
      if (expectedPosition !== null) {
        const drift = Math.abs(event.mediaTime - expectedPosition);
        if (drift > this.config.timingToleranceSeconds) {
          discrepancies.push({
            type: 'timing_drift',
            eventType: event.type,
            slotId: slot.slotId,
            details: `${event.type} fired at ${event.mediaTime}s, expected ${expectedPosition}s (drift: ${drift.toFixed(2)}s)`,
            severity: drift > 2 ? 'warning' : 'info',
          });
        }
      }
    }
  }

  /**
   * Get expected position for a quartile event
   */
  private getExpectedPosition(
    eventType: TrackingEventType,
    duration: number
  ): number | null {
    switch (eventType) {
      case 'firstQuartile':
        return duration * 0.25;
      case 'midpoint':
        return duration * 0.5;
      case 'thirdQuartile':
        return duration * 0.75;
      case 'complete':
        return duration;
      default:
        return null;
    }
  }

  /**
   * Get deterministic quartile status
   */
  getDeterministicStatus(slotId: string): {
    expectedQuartiles: QuartilePosition[];
    firedQuartiles: TrackingEventType[];
    remainingQuartiles: TrackingEventType[];
  } {
    const state = this.quartileStates.get(slotId);
    if (!state) {
      return {
        expectedQuartiles: [],
        firedQuartiles: [],
        remainingQuartiles: [],
      };
    }

    return {
      expectedQuartiles: state.positions,
      firedQuartiles: Array.from(state.fired),
      remainingQuartiles: state.positions
        .filter((q) => !state.fired.has(q.type))
        .map((q) => q.type),
    };
  }

  /**
   * Clean up old pods
   */
  cleanup(): void {
    const maxAge = this.config.maxAgeMinutes * 60 * 1000;
    const now = Date.now();

    for (const [podId, pod] of this.pods) {
      if (now - pod.startedAt.getTime() > maxAge) {
        this.pods.delete(podId);
        for (const slot of pod.slots) {
          this.quartileStates.delete(slot.slotId);
          this.receivedEvents.delete(slot.slotId);
        }
      }
    }
  }

  /**
   * Clear all state
   */
  clear(): void {
    this.pods.clear();
    this.quartileStates.clear();
    this.receivedEvents.clear();
  }
}
