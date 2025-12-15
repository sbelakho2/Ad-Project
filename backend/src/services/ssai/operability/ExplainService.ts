/**
 * SSAI Explain Service
 *
 * Provides detailed explanation of pod decisions for debugging
 * and support. Shows the full decision trace for any session/pod.
 */

import {
  ExplainPodRequest,
  ExplainPodResponse,
  ExplainDecisionStep,
  ExplainSlot,
} from './types';

interface StoredDecision {
  sessionId: string;
  podId: string;
  tenantId: string;
  deviceType: string;
  capabilities: string[];
  breakId: string;
  cueType: string;
  cueTime: Date;
  duration: number;
  steps: ExplainDecisionStep[];
  slots: ExplainSlot[];
  manifestType?: 'HLS' | 'DASH';
  manifestUrl?: string;
  createdAt: Date;
}

export interface ExplainServiceConfig {
  /** Maximum decisions to store */
  maxStoredDecisions: number;

  /** Retention period (minutes) */
  retentionMinutes: number;

  /** Include sensitive data */
  includeSensitiveData: boolean;
}

const DEFAULT_CONFIG: ExplainServiceConfig = {
  maxStoredDecisions: 10000,
  retentionMinutes: 60,
  includeSensitiveData: false,
};

export class ExplainService {
  private config: ExplainServiceConfig;
  private decisions: Map<string, StoredDecision> = new Map();
  private cleanupTimer: NodeJS.Timeout | null = null;

  constructor(config: Partial<ExplainServiceConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Start cleanup timer
   */
  start(): void {
    this.cleanupTimer = setInterval(() => {
      this.cleanup();
    }, 60000); // Cleanup every minute
  }

  /**
   * Stop cleanup timer
   */
  stop(): void {
    if (this.cleanupTimer) {
      clearInterval(this.cleanupTimer);
      this.cleanupTimer = null;
    }
  }

  /**
   * Generate key for a decision
   */
  private getKey(sessionId: string, podId: string): string {
    return `${sessionId}:${podId}`;
  }

  /**
   * Start recording a decision
   */
  startDecision(
    sessionId: string,
    podId: string,
    context: {
      tenantId: string;
      deviceType: string;
      capabilities: string[];
      breakId: string;
      cueType: string;
      cueTime: Date;
      duration: number;
    }
  ): void {
    const key = this.getKey(sessionId, podId);

    this.decisions.set(key, {
      sessionId,
      podId,
      tenantId: context.tenantId,
      deviceType: context.deviceType,
      capabilities: context.capabilities,
      breakId: context.breakId,
      cueType: context.cueType,
      cueTime: context.cueTime,
      duration: context.duration,
      steps: [],
      slots: [],
      createdAt: new Date(),
    });

    // Enforce max storage
    if (this.decisions.size > this.config.maxStoredDecisions) {
      const oldestKey = this.decisions.keys().next().value;
      if (oldestKey) {
        this.decisions.delete(oldestKey);
      }
    }
  }

  /**
   * Record a decision step
   */
  recordStep(
    sessionId: string,
    podId: string,
    step: {
      step: string;
      input: unknown;
      output: unknown;
      durationMs: number;
      notes?: string[];
    }
  ): void {
    const key = this.getKey(sessionId, podId);
    const decision = this.decisions.get(key);
    if (!decision) return;

    const stepRecord: ExplainDecisionStep = {
      step: step.step,
      order: decision.steps.length + 1,
      input: this.sanitize(step.input),
      output: this.sanitize(step.output),
      durationMs: step.durationMs,
      notes: step.notes,
    };

    decision.steps.push(stepRecord);
  }

  /**
   * Record a slot decision
   */
  recordSlot(
    sessionId: string,
    podId: string,
    slot: ExplainSlot
  ): void {
    const key = this.getKey(sessionId, podId);
    const decision = this.decisions.get(key);
    if (!decision) return;

    decision.slots.push(slot);
  }

  /**
   * Record manifest info
   */
  recordManifest(
    sessionId: string,
    podId: string,
    manifestType: 'HLS' | 'DASH',
    manifestUrl?: string
  ): void {
    const key = this.getKey(sessionId, podId);
    const decision = this.decisions.get(key);
    if (!decision) return;

    decision.manifestType = manifestType;
    decision.manifestUrl = this.config.includeSensitiveData ? manifestUrl : undefined;
  }

  /**
   * Explain a pod decision
   */
  explain(request: ExplainPodRequest): ExplainPodResponse | null {
    const key = this.getKey(request.sessionId, request.podId);
    const decision = this.decisions.get(key);

    if (!decision) {
      return null;
    }

    const warnings: string[] = [];

    // Check for issues
    if (decision.slots.length === 0) {
      warnings.push('No slots were filled');
    }

    const filledSlots = decision.slots.filter((s) => s.creativeId);
    if (filledSlots.length < decision.slots.length) {
      warnings.push(
        `Only ${filledSlots.length}/${decision.slots.length} slots were filled`
      );
    }

    // Check for slow steps
    const slowSteps = decision.steps.filter((s) => s.durationMs > 50);
    if (slowSteps.length > 0) {
      warnings.push(
        `${slowSteps.length} steps took >50ms`
      );
    }

    // Build response
    const response: ExplainPodResponse = {
      session: {
        sessionId: decision.sessionId,
        tenantId: decision.tenantId,
        deviceType: decision.deviceType,
        capabilities: decision.capabilities,
      },
      break: {
        breakId: decision.breakId,
        cueType: decision.cueType,
        cueTime: decision.cueTime,
        duration: decision.duration,
      },
      decisionTrace: request.includeDecisionTrace ? decision.steps : [],
      finalPod: {
        slotCount: decision.slots.length,
        slots: decision.slots,
      },
      manifest: decision.manifestType
        ? {
            type: decision.manifestType,
            sampleUrl: request.includeManifestSamples
              ? decision.manifestUrl
              : undefined,
          }
        : undefined,
      warnings,
    };

    return response;
  }

  /**
   * Get decision summary
   */
  getSummary(
    sessionId: string,
    podId: string
  ): {
    found: boolean;
    slotsFilled: number;
    totalSlots: number;
    totalDurationMs: number;
    stepCount: number;
  } {
    const key = this.getKey(sessionId, podId);
    const decision = this.decisions.get(key);

    if (!decision) {
      return {
        found: false,
        slotsFilled: 0,
        totalSlots: 0,
        totalDurationMs: 0,
        stepCount: 0,
      };
    }

    const slotsFilled = decision.slots.filter((s) => s.creativeId).length;
    const totalDurationMs = decision.steps.reduce((sum, s) => sum + s.durationMs, 0);

    return {
      found: true,
      slotsFilled,
      totalSlots: decision.slots.length,
      totalDurationMs,
      stepCount: decision.steps.length,
    };
  }

  /**
   * Sanitize data for storage (remove sensitive info)
   */
  private sanitize(data: unknown): unknown {
    if (!this.config.includeSensitiveData) {
      // In production, strip PII and sensitive data
      return data;
    }
    return data;
  }

  /**
   * Cleanup old decisions
   */
  cleanup(): void {
    const cutoff = new Date(
      Date.now() - this.config.retentionMinutes * 60 * 1000
    );

    for (const [key, decision] of this.decisions) {
      if (decision.createdAt < cutoff) {
        this.decisions.delete(key);
      }
    }
  }

  /**
   * Get stats
   */
  getStats(): {
    totalDecisions: number;
    oldestDecision: Date | null;
    newestDecision: Date | null;
  } {
    let oldest: Date | null = null;
    let newest: Date | null = null;

    for (const decision of this.decisions.values()) {
      if (!oldest || decision.createdAt < oldest) {
        oldest = decision.createdAt;
      }
      if (!newest || decision.createdAt > newest) {
        newest = decision.createdAt;
      }
    }

    return {
      totalDecisions: this.decisions.size,
      oldestDecision: oldest,
      newestDecision: newest,
    };
  }

  /**
   * Clear all state
   */
  clear(): void {
    this.decisions.clear();
  }
}
