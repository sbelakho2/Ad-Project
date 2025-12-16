/**
 * SSAI Safe Mode Manager
 *
 * Automatic degradation when system health deteriorates.
 * Reduces load and complexity during incidents.
 */

export interface SafeModeManagerConfig {
  /** Trigger when error rate exceeds this fraction (e.g. 0.1 = 10%). */
  errorRateThreshold: number;

  /** Trigger when p99 latency exceeds this (ms). */
  latencyP99ThresholdMs: number;

  /** Rolling evaluation window. */
  evaluationWindowMs: number;

  /** Allow auto-recovery when metrics return to normal. */
  autoRecoveryEnabled: boolean;
}

export interface SafeModeMetric {
  success: boolean;
  latencyMs: number;
}

export interface SafeModeDegradedBehavior {
  skipPodDecisioning: boolean;
  usePrebuiltManifests: boolean;
  reducedPartnerTimeoutMs: number;
  disableNonCritical: boolean;
}

const DEFAULT_BEHAVIOR: SafeModeDegradedBehavior = {
  skipPodDecisioning: true,
  usePrebuiltManifests: true,
  reducedPartnerTimeoutMs: 200,
  disableNonCritical: true,
};

type StoredMetric = SafeModeMetric & { ts: number };

export class SafeModeManager {
  private readonly config: SafeModeManagerConfig;
  private readonly behavior: SafeModeDegradedBehavior;
  private metrics: StoredMetric[] = [];
  private active = false;
  private reason: string | null = null;

  constructor(config: SafeModeManagerConfig, behavior: Partial<SafeModeDegradedBehavior> = {}) {
    this.config = config;
    this.behavior = { ...DEFAULT_BEHAVIOR, ...behavior };
  }

  recordMetric(metric: SafeModeMetric): void {
    const now = Date.now();
    this.metrics.push({ ...metric, ts: now });
    this.prune(now);
    this.evaluate();
  }

  isActive(): boolean {
    return this.active;
  }

  forceActivate(reason: string): void {
    this.active = true;
    this.reason = reason;
  }

  forceDeactivate(): void {
    this.active = false;
    this.reason = null;
  }

  getDegradedBehavior(): SafeModeDegradedBehavior {
    return this.behavior;
  }

  getReason(): string | null {
    return this.reason;
  }

  private prune(now: number): void {
    const cutoff = now - this.config.evaluationWindowMs;
    while (this.metrics.length && this.metrics[0].ts < cutoff) {
      this.metrics.shift();
    }
  }

  private evaluate(): void {
    if (this.metrics.length === 0) return;

    const total = this.metrics.length;
    const failures = this.metrics.filter((m) => !m.success).length;
    const errorRate = failures / total;

    const latencies = this.metrics.map((m) => m.latencyMs).sort((a, b) => a - b);
    const p99Index = Math.min(latencies.length - 1, Math.floor(latencies.length * 0.99));
    const latencyP99 = latencies[p99Index] ?? 0;

    const shouldActivate =
      errorRate > this.config.errorRateThreshold ||
      latencyP99 > this.config.latencyP99ThresholdMs;

    if (shouldActivate) {
      this.active = true;
      this.reason = `threshold_exceeded(errorRate=${errorRate.toFixed(4)},latencyP99=${latencyP99})`;
      return;
    }

    if (this.active && this.config.autoRecoveryEnabled) {
      this.active = false;
      this.reason = null;
    }
  }
}
