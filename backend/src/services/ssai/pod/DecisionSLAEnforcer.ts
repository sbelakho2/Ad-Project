/**
 * Decision SLA Enforcer
 *
 * Enforces hard deadlines for pod decisions and manages
 * safe mode for degraded conditions.
 */

import {
  DecisionDeadline,
  DecisionSLAConfig,
  PodDecisionRequest,
  PodDecisionResponse,
  SlotDecision,
} from './types';

export interface PartnerCallResult {
  partnerId: string;
  success: boolean;
  latencyMs: number;
  timedOut: boolean;
  error?: string;
}

export interface SLAMetrics {
  decisionsTotal: number;
  decisionsMissedDeadline: number;
  averageDecisionTimeMs: number;
  p95DecisionTimeMs: number;
  p99DecisionTimeMs: number;
  safeModeActivations: number;
}

/**
 * Decision SLA Enforcer ensures all pod decisions complete
 * before the hard deadline, enabling slate fallback if needed.
 */
export class DecisionSLAEnforcer {
  private config: DecisionSLAConfig;
  private safeModeActive: boolean = false;
  private safeModeActivatedAt?: Date;
  private decisionTimes: number[] = [];
  private metrics: SLAMetrics = {
    decisionsTotal: 0,
    decisionsMissedDeadline: 0,
    averageDecisionTimeMs: 0,
    p95DecisionTimeMs: 0,
    p99DecisionTimeMs: 0,
    safeModeActivations: 0,
  };

  constructor(config?: Partial<DecisionSLAConfig>) {
    this.config = {
      maxDecisionTimeMs: 250,
      partnerTimeoutMs: 150,
      safeModeThresholdMs: 200,
      safeModeMaxPartners: 3,
      circuitBreakerEnabled: true,
      circuitBreakerErrorThreshold: 0.5,
      circuitBreakerTimeoutThreshold: 0.3,
      circuitBreakerCooldownMs: 30000,
      ...config,
    };
  }

  /**
   * Calculate deadline for a pod decision.
   */
  calculateDeadline(request: PodDecisionRequest): DecisionDeadline {
    const requestedDeadline = new Date(request.deadline);
    const now = Date.now();

    // Calculate buffer (10% of available time, minimum 20ms)
    const availableTimeMs = requestedDeadline.getTime() - now;
    const bufferMs = Math.max(20, Math.min(50, availableTimeMs * 0.1));

    // Effective deadline is requested deadline minus buffer
    const effectiveDeadline = new Date(requestedDeadline.getTime() - bufferMs);

    return {
      deadline: effectiveDeadline,
      bufferMs,
      safeMode: this.safeModeActive,
      reducedPartners: this.safeModeActive ? this.getReducedPartnerSet() : undefined,
    };
  }

  /**
   * Check if we're still within deadline.
   */
  isWithinDeadline(deadline: DecisionDeadline): boolean {
    return Date.now() < deadline.deadline.getTime();
  }

  /**
   * Get remaining time until deadline.
   */
  getRemainingTimeMs(deadline: DecisionDeadline): number {
    return Math.max(0, deadline.deadline.getTime() - Date.now());
  }

  /**
   * Execute decision with deadline enforcement.
   *
   * This wrapper ensures we never exceed the deadline,
   * using slate fallback if necessary.
   */
  async executeWithDeadline<T>(
    deadline: DecisionDeadline,
    operation: () => Promise<T>,
    fallback: () => T
  ): Promise<{ result: T; usedFallback: boolean; latencyMs: number }> {
    const startTime = Date.now();
    const remainingMs = this.getRemainingTimeMs(deadline);

    if (remainingMs <= 0) {
      // Already past deadline, use fallback immediately
      return {
        result: fallback(),
        usedFallback: true,
        latencyMs: 0,
      };
    }

    try {
      // Create timeout promise
      const timeoutPromise = new Promise<never>((_, reject) => {
        setTimeout(() => reject(new Error('Decision timeout')), remainingMs);
      });

      // Race operation against timeout
      const result = await Promise.race([operation(), timeoutPromise]);

      const latencyMs = Date.now() - startTime;
      this.recordDecisionTime(latencyMs);

      return {
        result,
        usedFallback: false,
        latencyMs,
      };
    } catch (error) {
      const latencyMs = Date.now() - startTime;

      // Use fallback on any error
      return {
        result: fallback(),
        usedFallback: true,
        latencyMs,
      };
    }
  }

  /**
   * Execute partner calls with individual timeouts.
   */
  async executePartnerCalls<T>(
    partners: string[],
    deadline: DecisionDeadline,
    callPartner: (partnerId: string) => Promise<T>
  ): Promise<Map<string, { result?: T; error?: string; latencyMs: number }>> {
    const results = new Map<
      string,
      { result?: T; error?: string; latencyMs: number }
    >();

    // Limit partners in safe mode
    const partnersToCall = deadline.safeMode
      ? partners.slice(0, this.config.safeModeMaxPartners)
      : partners;

    // Calculate per-partner timeout
    const remainingMs = this.getRemainingTimeMs(deadline);
    const partnerTimeout = Math.min(this.config.partnerTimeoutMs, remainingMs);

    // Execute all partner calls in parallel
    const calls = partnersToCall.map(async (partnerId) => {
      const startTime = Date.now();

      try {
        const timeoutPromise = new Promise<never>((_, reject) => {
          setTimeout(() => reject(new Error('Partner timeout')), partnerTimeout);
        });

        const result = await Promise.race([callPartner(partnerId), timeoutPromise]);
        const latencyMs = Date.now() - startTime;

        results.set(partnerId, { result, latencyMs });
      } catch (error) {
        const latencyMs = Date.now() - startTime;
        results.set(partnerId, {
          error: error instanceof Error ? error.message : 'Unknown error',
          latencyMs,
        });
      }
    });

    await Promise.allSettled(calls);
    return results;
  }

  /**
   * Create slate fallback response.
   */
  createSlateFallback(request: PodDecisionRequest): PodDecisionResponse {
    const slotDecisions: SlotDecision[] = request.slots.map((slot) => ({
      slotId: slot.slotId,
      filled: false,
      slate: slot.allowSlate
        ? {
            slateId: 'default-slate',
            mediaFile: {
              uri: '/slate/default.ts',
              delivery: 'streaming',
              container: request.formatRequirements.container,
              bitrate: 2000,
              width: 1920,
              height: 1080,
            },
            duration: slot.duration,
          }
        : undefined,
      reason: 'Deadline exceeded, using slate fallback',
    }));

    return {
      podId: request.podId,
      breakId: request.breakId,
      slotDecisions,
      decisionTimeMs: 0,
      metDeadline: false,
      warnings: ['Decision fell back to slate due to deadline'],
    };
  }

  /**
   * Record decision time for metrics and safe mode detection.
   */
  private recordDecisionTime(latencyMs: number): void {
    this.decisionTimes.push(latencyMs);
    this.metrics.decisionsTotal++;

    // Keep only last 100 samples
    if (this.decisionTimes.length > 100) {
      this.decisionTimes.shift();
    }

    // Update metrics
    this.updateMetrics();

    // Check if we should enter/exit safe mode
    this.evaluateSafeMode();
  }

  /**
   * Update metrics from decision times.
   */
  private updateMetrics(): void {
    if (this.decisionTimes.length === 0) {
      return;
    }

    const sorted = [...this.decisionTimes].sort((a, b) => a - b);
    const sum = sorted.reduce((a, b) => a + b, 0);

    this.metrics.averageDecisionTimeMs = sum / sorted.length;
    this.metrics.p95DecisionTimeMs = sorted[Math.floor(sorted.length * 0.95)] || 0;
    this.metrics.p99DecisionTimeMs = sorted[Math.floor(sorted.length * 0.99)] || 0;
  }

  /**
   * Evaluate whether to enter or exit safe mode.
   */
  private evaluateSafeMode(): void {
    // Enter safe mode if p95 exceeds threshold
    if (
      !this.safeModeActive &&
      this.metrics.p95DecisionTimeMs > this.config.safeModeThresholdMs
    ) {
      this.activateSafeMode();
    }

    // Exit safe mode if p95 drops below threshold and we've been in safe mode for a while
    if (
      this.safeModeActive &&
      this.metrics.p95DecisionTimeMs < this.config.safeModeThresholdMs * 0.8 &&
      this.safeModeActivatedAt
    ) {
      const safeModeMinDurationMs = 60000; // Stay in safe mode for at least 1 minute
      if (
        Date.now() - this.safeModeActivatedAt.getTime() >
        safeModeMinDurationMs
      ) {
        this.deactivateSafeMode();
      }
    }
  }

  /**
   * Activate safe mode.
   */
  activateSafeMode(): void {
    if (!this.safeModeActive) {
      this.safeModeActive = true;
      this.safeModeActivatedAt = new Date();
      this.metrics.safeModeActivations++;
      console.warn('[SLA] Safe mode activated due to high latency');
    }
  }

  /**
   * Deactivate safe mode.
   */
  deactivateSafeMode(): void {
    if (this.safeModeActive) {
      this.safeModeActive = false;
      this.safeModeActivatedAt = undefined;
      console.info('[SLA] Safe mode deactivated');
    }
  }

  /**
   * Force safe mode (manual trigger).
   */
  forceSafeMode(reason: string): void {
    this.safeModeActive = true;
    this.safeModeActivatedAt = new Date();
    this.metrics.safeModeActivations++;
    console.warn(`[SLA] Safe mode forced: ${reason}`);
  }

  /**
   * Get reduced partner set for safe mode.
   */
  private getReducedPartnerSet(): string[] {
    // In a real implementation, this would return the most reliable partners
    return ['partner-1', 'partner-2', 'partner-3'].slice(
      0,
      this.config.safeModeMaxPartners
    );
  }

  /**
   * Record missed deadline.
   */
  recordMissedDeadline(): void {
    this.metrics.decisionsMissedDeadline++;
  }

  /**
   * Get current metrics.
   */
  getMetrics(): SLAMetrics {
    return { ...this.metrics };
  }

  /**
   * Check if safe mode is active.
   */
  isSafeModeActive(): boolean {
    return this.safeModeActive;
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<DecisionSLAConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration.
   */
  getConfig(): DecisionSLAConfig {
    return { ...this.config };
  }

  /**
   * Reset metrics (for testing).
   */
  resetMetrics(): void {
    this.decisionTimes = [];
    this.metrics = {
      decisionsTotal: 0,
      decisionsMissedDeadline: 0,
      averageDecisionTimeMs: 0,
      p95DecisionTimeMs: 0,
      p99DecisionTimeMs: 0,
      safeModeActivations: 0,
    };
  }
}

export default DecisionSLAEnforcer;
