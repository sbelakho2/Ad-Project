/**
 * SSAI Live Reconciler
 *
 * Handles tracking reconciliation for live streaming with drift tolerance.
 * Live streams can have timing drift between server and client,
 * so this reconciler is more lenient.
 */

import {
  TrackingEvent,
  TrackingEventType,
  ReconciliationResult,
  SlotReconciliation,
  Discrepancy,
} from './types';

interface LivePodInfo {
  podId: string;
  slots: LiveSlotInfo[];
  cueTime: Date;
  podDuration: number;
  expectedEndTime: Date;
}

interface LiveSlotInfo {
  slotId: string;
  creativeId: string;
  duration: number;
  startTime: Date;
  expectedEndTime: Date;
}

interface DriftState {
  samples: number[];
  averageDrift: number;
  maxDrift: number;
  lastUpdate: Date;
}

export interface LiveReconcilerConfig {
  /** Maximum allowed drift before warning (seconds) */
  maxDriftSeconds: number;

  /** Time window for drift calculation (samples) */
  driftWindowSize: number;

  /** Grace period after pod end for late events (seconds) */
  gracePeriodSeconds: number;

  /** Maximum age for reconciliation (minutes) */
  maxAgeMinutes: number;

  /** Enable real-time reconciliation */
  realTimeReconciliation: boolean;
}

const DEFAULT_CONFIG: LiveReconcilerConfig = {
  maxDriftSeconds: 3,
  driftWindowSize: 10,
  gracePeriodSeconds: 10,
  maxAgeMinutes: 30,
  realTimeReconciliation: true,
};

export class LiveReconciler {
  private config: LiveReconcilerConfig;
  private pods: Map<string, LivePodInfo> = new Map();
  private driftStates: Map<string, DriftState> = new Map();
  private receivedEvents: Map<string, TrackingEvent[]> = new Map();
  private pendingReconciliations: Map<string, NodeJS.Timeout> = new Map();

  constructor(config: Partial<LiveReconcilerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Register a live pod for reconciliation
   */
  registerPod(
    podId: string,
    slots: Array<{ slotId: string; creativeId: string; duration: number }>,
    cueTime: Date
  ): void {
    let currentTime = cueTime.getTime();
    const liveSlots: LiveSlotInfo[] = [];

    for (const slot of slots) {
      const startTime = new Date(currentTime);
      currentTime += slot.duration * 1000;
      const endTime = new Date(currentTime);

      liveSlots.push({
        slotId: slot.slotId,
        creativeId: slot.creativeId,
        duration: slot.duration,
        startTime,
        expectedEndTime: endTime,
      });

      this.receivedEvents.set(slot.slotId, []);
      this.driftStates.set(slot.slotId, {
        samples: [],
        averageDrift: 0,
        maxDrift: 0,
        lastUpdate: new Date(),
      });
    }

    const podDuration = slots.reduce((sum, s) => sum + s.duration, 0);
    const expectedEndTime = new Date(cueTime.getTime() + podDuration * 1000);

    this.pods.set(podId, {
      podId,
      slots: liveSlots,
      cueTime,
      podDuration,
      expectedEndTime,
    });

    // Schedule reconciliation after grace period
    if (this.config.realTimeReconciliation) {
      const delay = (podDuration + this.config.gracePeriodSeconds) * 1000;
      const timeout = setTimeout(() => {
        this.reconcile(podId);
        this.pendingReconciliations.delete(podId);
      }, delay);
      this.pendingReconciliations.set(podId, timeout);
    }
  }

  /**
   * Record a received tracking event
   */
  recordEvent(event: TrackingEvent): void {
    const events = this.receivedEvents.get(event.slotId);
    if (!events) return;

    events.push(event);

    // Update drift calculation
    this.updateDrift(event);
  }

  /**
   * Update drift calculation based on event timing
   */
  private updateDrift(event: TrackingEvent): void {
    const driftState = this.driftStates.get(event.slotId);
    if (!driftState) return;

    // Find the slot
    for (const pod of this.pods.values()) {
      const slot = pod.slots.find((s) => s.slotId === event.slotId);
      if (!slot) continue;

      // Calculate expected time based on event type
      const expectedTime = this.getExpectedEventTime(event.type, slot);
      if (expectedTime) {
        const drift = event.timestamp.getTime() - expectedTime.getTime();
        const driftSeconds = drift / 1000;

        driftState.samples.push(driftSeconds);
        if (driftState.samples.length > this.config.driftWindowSize) {
          driftState.samples.shift();
        }

        driftState.averageDrift =
          driftState.samples.reduce((a, b) => a + b, 0) / driftState.samples.length;
        driftState.maxDrift = Math.max(
          driftState.maxDrift,
          Math.abs(driftSeconds)
        );
        driftState.lastUpdate = new Date();
      }
      break;
    }
  }

  /**
   * Get expected event time for a slot
   */
  private getExpectedEventTime(
    eventType: TrackingEventType,
    slot: LiveSlotInfo
  ): Date | null {
    const startMs = slot.startTime.getTime();
    const durationMs = slot.duration * 1000;

    switch (eventType) {
      case 'impression':
      case 'start':
        return slot.startTime;
      case 'firstQuartile':
        return new Date(startMs + durationMs * 0.25);
      case 'midpoint':
        return new Date(startMs + durationMs * 0.5);
      case 'thirdQuartile':
        return new Date(startMs + durationMs * 0.75);
      case 'complete':
        return slot.expectedEndTime;
      default:
        return null;
    }
  }

  /**
   * Get current drift for a slot
   */
  getDrift(slotId: string): DriftState | null {
    return this.driftStates.get(slotId) || null;
  }

  /**
   * Check if events are within drift tolerance
   */
  isWithinTolerance(slotId: string): boolean {
    const driftState = this.driftStates.get(slotId);
    if (!driftState) return true;
    return Math.abs(driftState.averageDrift) <= this.config.maxDriftSeconds;
  }

  /**
   * Reconcile a live pod
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
      const events = this.receivedEvents.get(slot.slotId) || [];
      const driftState = this.driftStates.get(slot.slotId);

      // Expected events for live
      const expectedEvents: TrackingEventType[] = [
        'impression',
        'start',
        'firstQuartile',
        'midpoint',
        'thirdQuartile',
        'complete',
      ];

      // Received events
      const receivedEventTypes = new Set(events.map((e) => e.type));
      const receivedEvents = Array.from(receivedEventTypes);

      // Calculate missing and extra (with tolerance for live)
      const missingEvents = expectedEvents.filter(
        (e) => !receivedEventTypes.has(e)
      );
      const extraEvents = receivedEvents.filter(
        (e) => !expectedEvents.includes(e)
      ) as TrackingEventType[];

      // Check completion
      const completed = receivedEventTypes.has('complete');
      if (completed) completedSlots++;

      // Check fill
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
        // Live has more tolerance - only warn on critical events
        const severity =
          missing === 'impression' || missing === 'complete' ? 'warning' : 'info';
        discrepancies.push({
          type: 'missing_event',
          eventType: missing,
          slotId: slot.slotId,
          details: `Expected ${missing} event was not received (live stream)`,
          severity,
        });
      }

      // Check drift discrepancies
      if (driftState && Math.abs(driftState.averageDrift) > this.config.maxDriftSeconds) {
        discrepancies.push({
          type: 'timing_drift',
          eventType: 'start',
          slotId: slot.slotId,
          details: `Average drift of ${driftState.averageDrift.toFixed(2)}s exceeds threshold of ${this.config.maxDriftSeconds}s`,
          severity: 'warning',
        });
      }

      // Check for duplicate events
      this.checkDuplicates(events, slot.slotId, discrepancies);
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
   * Check for duplicate events
   */
  private checkDuplicates(
    events: TrackingEvent[],
    slotId: string,
    discrepancies: Discrepancy[]
  ): void {
    const eventCounts = new Map<TrackingEventType, number>();

    for (const event of events) {
      eventCounts.set(event.type, (eventCounts.get(event.type) || 0) + 1);
    }

    for (const [eventType, count] of eventCounts) {
      if (count > 1) {
        discrepancies.push({
          type: 'duplicate_event',
          eventType,
          slotId,
          details: `${eventType} event received ${count} times`,
          severity: 'info',
        });
      }
    }
  }

  /**
   * Get real-time status for a pod
   */
  getRealTimeStatus(podId: string): {
    isActive: boolean;
    elapsedSeconds: number;
    remainingSeconds: number;
    currentSlot: LiveSlotInfo | null;
    eventsReceived: number;
    driftStatus: 'normal' | 'warning' | 'critical';
  } | null {
    const pod = this.pods.get(podId);
    if (!pod) return null;

    const now = Date.now();
    const elapsedSeconds = (now - pod.cueTime.getTime()) / 1000;
    const remainingSeconds = Math.max(0, pod.podDuration - elapsedSeconds);
    const isActive = remainingSeconds > 0;

    // Find current slot
    let currentSlot: LiveSlotInfo | null = null;
    for (const slot of pod.slots) {
      if (now >= slot.startTime.getTime() && now < slot.expectedEndTime.getTime()) {
        currentSlot = slot;
        break;
      }
    }

    // Count events
    let eventsReceived = 0;
    for (const slot of pod.slots) {
      const events = this.receivedEvents.get(slot.slotId) || [];
      eventsReceived += events.length;
    }

    // Calculate drift status
    let driftStatus: 'normal' | 'warning' | 'critical' = 'normal';
    for (const slot of pod.slots) {
      const driftState = this.driftStates.get(slot.slotId);
      if (driftState) {
        const drift = Math.abs(driftState.averageDrift);
        if (drift > this.config.maxDriftSeconds * 2) {
          driftStatus = 'critical';
          break;
        } else if (drift > this.config.maxDriftSeconds) {
          driftStatus = 'warning';
        }
      }
    }

    return {
      isActive,
      elapsedSeconds,
      remainingSeconds,
      currentSlot,
      eventsReceived,
      driftStatus,
    };
  }

  /**
   * Clean up old pods
   */
  cleanup(): void {
    const maxAge = this.config.maxAgeMinutes * 60 * 1000;
    const now = Date.now();

    for (const [podId, pod] of this.pods) {
      // Clean up if past expected end + grace period + max age
      const cleanupTime =
        pod.expectedEndTime.getTime() +
        this.config.gracePeriodSeconds * 1000 +
        maxAge;

      if (now > cleanupTime) {
        // Cancel pending reconciliation
        const timeout = this.pendingReconciliations.get(podId);
        if (timeout) {
          clearTimeout(timeout);
          this.pendingReconciliations.delete(podId);
        }

        // Clean up state
        this.pods.delete(podId);
        for (const slot of pod.slots) {
          this.receivedEvents.delete(slot.slotId);
          this.driftStates.delete(slot.slotId);
        }
      }
    }
  }

  /**
   * Clear all state
   */
  clear(): void {
    for (const timeout of this.pendingReconciliations.values()) {
      clearTimeout(timeout);
    }
    this.pendingReconciliations.clear();
    this.pods.clear();
    this.receivedEvents.clear();
    this.driftStates.clear();
  }
}
