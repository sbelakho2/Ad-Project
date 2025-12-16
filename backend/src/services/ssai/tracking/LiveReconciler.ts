/**
 * SSAI Live Reconciler (test-facing)
 */

export interface LiveReconcilerConfig {
  driftToleranceMs: number;
  gracePeriodMs: number;
}

export interface LiveStatus {
  currentTime: number;
  liveEdge: number;
  bufferLength: number;
  drift: number;
  isWithinTolerance: boolean;
  updatedAt: number;
}

export class LiveReconciler {
  private readonly config: LiveReconcilerConfig;
  private statuses: Map<string, LiveStatus> = new Map();

  constructor(config: LiveReconcilerConfig) {
    this.config = config;
  }

  updateStatus(sessionId: string, status: { currentTime: number; liveEdge: number; bufferLength: number }): void {
    const drift = status.liveEdge - status.currentTime;
    const isWithinTolerance = Math.abs(drift) * 1000 <= this.config.driftToleranceMs;

    this.statuses.set(sessionId, {
      currentTime: status.currentTime,
      liveEdge: status.liveEdge,
      bufferLength: status.bufferLength,
      drift,
      isWithinTolerance,
      updatedAt: Date.now(),
    });
  }

  getStatus(sessionId: string): LiveStatus {
    return (
      this.statuses.get(sessionId) ?? {
        currentTime: 0,
        liveEdge: 0,
        bufferLength: 0,
        drift: 0,
        isWithinTolerance: true,
        updatedAt: 0,
      }
    );
  }

  shouldFireEvent(
    eventType: string,
    timing: { expectedTime: number; actualTime: number }
  ): { shouldFire: boolean; reason: string } {
    const diffMs = Math.abs(timing.actualTime - timing.expectedTime) * 1000;
    if (diffMs <= this.config.gracePeriodMs) {
      return { shouldFire: true, reason: 'within_grace_period' };
    }

    return {
      shouldFire: false,
      reason: `${eventType} outside grace period (diff=${diffMs}ms)`,
    };
  }
}
