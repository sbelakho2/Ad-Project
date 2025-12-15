/**
 * SSAI Safe Mode Manager
 *
 * Automatic degradation when system health deteriorates.
 * Reduces load and complexity during incidents.
 */

import {
  SafeModeConfig,
  SafeModeBehavior,
  SafeModeStatus,
} from './types';

export interface SafeModeManagerConfig {
  /** Check interval (seconds) */
  checkIntervalSeconds: number;

  /** Metrics source refresh interval (seconds) */
  metricsRefreshSeconds: number;

  /** Number of consecutive healthy checks for recovery */
  recoveryChecksRequired: number;
}

const DEFAULT_CONFIG: SafeModeManagerConfig = {
  checkIntervalSeconds: 10,
  metricsRefreshSeconds: 5,
  recoveryChecksRequired: 3,
};

const DEFAULT_SAFE_MODE_CONFIG: SafeModeConfig = {
  active: false,
  errorRateThreshold: 10, // 10% error rate
  latencyThreshold: 500, // 500ms p99
  minimumSamples: 100,
  autoRecovery: true,
  recoveryCheckSeconds: 60,
  behavior: {
    skipDecisions: false,
    useSlate: true,
    reducedTimeout: 100,
    disableNonCritical: true,
  },
};

interface MetricsSample {
  timestamp: Date;
  requestCount: number;
  errorCount: number;
  latencyP99: number;
}

export class SafeModeManager {
  private config: SafeModeManagerConfig;
  private safeModeConfig: SafeModeConfig;
  private isActive: boolean = false;
  private activatedAt: Date | null = null;
  private activationReason: string | null = null;
  private checkTimer: NodeJS.Timeout | null = null;
  private recoveryChecksPassed: number = 0;
  private lastRecoveryCheck: Date | null = null;
  private metricsSamples: MetricsSample[] = [];
  private readonly maxSamples = 60;

  constructor(
    config: Partial<SafeModeManagerConfig> = {},
    safeModeConfig: Partial<SafeModeConfig> = {}
  ) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.safeModeConfig = { ...DEFAULT_SAFE_MODE_CONFIG, ...safeModeConfig };
  }

  /**
   * Start the manager
   */
  start(): void {
    this.checkTimer = setInterval(() => {
      this.check();
    }, this.config.checkIntervalSeconds * 1000);
  }

  /**
   * Stop the manager
   */
  stop(): void {
    if (this.checkTimer) {
      clearInterval(this.checkTimer);
      this.checkTimer = null;
    }
  }

  /**
   * Record a metrics sample
   */
  recordMetrics(sample: MetricsSample): void {
    this.metricsSamples.push(sample);
    if (this.metricsSamples.length > this.maxSamples) {
      this.metricsSamples.shift();
    }
  }

  /**
   * Get current metrics
   */
  getCurrentMetrics(): {
    errorRate: number;
    latencyP99: number;
    sampleCount: number;
  } {
    if (this.metricsSamples.length === 0) {
      return {
        errorRate: 0,
        latencyP99: 0,
        sampleCount: 0,
      };
    }

    // Calculate aggregates over recent samples
    const recentSamples = this.metricsSamples.slice(-10);
    let totalRequests = 0;
    let totalErrors = 0;
    const latencies: number[] = [];

    for (const sample of recentSamples) {
      totalRequests += sample.requestCount;
      totalErrors += sample.errorCount;
      latencies.push(sample.latencyP99);
    }

    const errorRate = totalRequests > 0 ? (totalErrors / totalRequests) * 100 : 0;
    const latencyP99 = latencies.length > 0
      ? latencies.sort((a, b) => b - a)[0]
      : 0;

    return {
      errorRate,
      latencyP99,
      sampleCount: totalRequests,
    };
  }

  /**
   * Check if safe mode should be activated/deactivated
   */
  check(): void {
    const metrics = this.getCurrentMetrics();

    if (!this.isActive) {
      // Check if we should activate
      if (this.shouldActivate(metrics)) {
        this.activate(`Metrics exceeded thresholds: errorRate=${metrics.errorRate.toFixed(2)}%, latencyP99=${metrics.latencyP99}ms`);
      }
    } else {
      // Check if we should recover
      if (this.safeModeConfig.autoRecovery) {
        this.checkRecovery(metrics);
      }
    }
  }

  /**
   * Check if safe mode should be activated
   */
  private shouldActivate(metrics: {
    errorRate: number;
    latencyP99: number;
    sampleCount: number;
  }): boolean {
    // Need minimum samples
    if (metrics.sampleCount < this.safeModeConfig.minimumSamples) {
      return false;
    }

    // Check error rate
    if (metrics.errorRate > this.safeModeConfig.errorRateThreshold) {
      return true;
    }

    // Check latency
    if (metrics.latencyP99 > this.safeModeConfig.latencyThreshold) {
      return true;
    }

    return false;
  }

  /**
   * Check if we can recover from safe mode
   */
  private checkRecovery(metrics: {
    errorRate: number;
    latencyP99: number;
    sampleCount: number;
  }): void {
    const now = new Date();

    // Check if enough time has passed since last check
    if (this.lastRecoveryCheck) {
      const elapsed = (now.getTime() - this.lastRecoveryCheck.getTime()) / 1000;
      if (elapsed < this.safeModeConfig.recoveryCheckSeconds) {
        return;
      }
    }

    this.lastRecoveryCheck = now;

    // Check if metrics are healthy
    const isHealthy =
      metrics.errorRate < this.safeModeConfig.errorRateThreshold * 0.5 &&
      metrics.latencyP99 < this.safeModeConfig.latencyThreshold * 0.5;

    if (isHealthy) {
      this.recoveryChecksPassed++;
      if (this.recoveryChecksPassed >= this.config.recoveryChecksRequired) {
        this.deactivate('Auto-recovery: metrics healthy');
      }
    } else {
      // Reset recovery counter
      this.recoveryChecksPassed = 0;
    }
  }

  /**
   * Manually activate safe mode
   */
  activate(reason: string): void {
    if (this.isActive) return;

    this.isActive = true;
    this.activatedAt = new Date();
    this.activationReason = reason;
    this.recoveryChecksPassed = 0;

    console.warn(`[SafeMode] ACTIVATED: ${reason}`);
  }

  /**
   * Manually deactivate safe mode
   */
  deactivate(reason: string): void {
    if (!this.isActive) return;

    this.isActive = false;
    this.activatedAt = null;
    this.activationReason = null;
    this.recoveryChecksPassed = 0;
    this.lastRecoveryCheck = null;

    console.info(`[SafeMode] DEACTIVATED: ${reason}`);
  }

  /**
   * Get current status
   */
  getStatus(): SafeModeStatus {
    const metrics = this.getCurrentMetrics();

    return {
      isActive: this.isActive,
      reason: this.activationReason || undefined,
      activatedAt: this.activatedAt || undefined,
      currentMetrics: metrics,
      recoveryStatus: this.isActive
        ? {
            checking: this.safeModeConfig.autoRecovery,
            lastCheck: this.lastRecoveryCheck || new Date(),
            checksPassed: this.recoveryChecksPassed,
            checksRequired: this.config.recoveryChecksRequired,
          }
        : undefined,
    };
  }

  /**
   * Get safe mode behavior
   */
  getBehavior(): SafeModeBehavior {
    return this.safeModeConfig.behavior;
  }

  /**
   * Check if a specific feature should be disabled
   */
  shouldSkipDecisions(): boolean {
    return this.isActive && this.safeModeConfig.behavior.skipDecisions;
  }

  /**
   * Check if we should use slate
   */
  shouldUseSlate(): boolean {
    return this.isActive && this.safeModeConfig.behavior.useSlate;
  }

  /**
   * Get reduced timeout (if any)
   */
  getReducedTimeout(): number | null {
    if (!this.isActive) return null;
    return this.safeModeConfig.behavior.reducedTimeout || null;
  }

  /**
   * Check if non-critical features should be disabled
   */
  shouldDisableNonCritical(): boolean {
    return this.isActive && this.safeModeConfig.behavior.disableNonCritical;
  }

  /**
   * Update safe mode configuration
   */
  updateConfig(config: Partial<SafeModeConfig>): void {
    this.safeModeConfig = { ...this.safeModeConfig, ...config };
  }

  /**
   * Clear all state
   */
  clear(): void {
    this.isActive = false;
    this.activatedAt = null;
    this.activationReason = null;
    this.recoveryChecksPassed = 0;
    this.lastRecoveryCheck = null;
    this.metricsSamples = [];
  }
}
