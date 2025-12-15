/**
 * SSAI SLI Metrics Manager
 *
 * Tracks Service Level Indicators for SSAI operations.
 * Provides real-time health monitoring and error budget tracking.
 */

import { SLIDefinition, SLIMetric, SLIReport } from './types';

export interface SLIManagerConfig {
  /** Report generation interval (seconds) */
  reportIntervalSeconds: number;

  /** Metrics retention period (hours) */
  retentionHours: number;

  /** Enable error budget tracking */
  enableErrorBudget: boolean;

  /** Monthly error budget (for SLO) */
  monthlyErrorBudgetPercent: number;
}

const DEFAULT_CONFIG: SLIManagerConfig = {
  reportIntervalSeconds: 60,
  retentionHours: 24,
  enableErrorBudget: true,
  monthlyErrorBudgetPercent: 0.1, // 99.9% SLO = 0.1% error budget
};

interface MetricDataPoint {
  timestamp: Date;
  value: number;
}

interface MetricState {
  definition: SLIDefinition;
  dataPoints: MetricDataPoint[];
  lastValue: number;
  trend: 'improving' | 'stable' | 'degrading';
}

export class SLIManager {
  private config: SLIManagerConfig;
  private metrics: Map<string, MetricState> = new Map();
  private errorBudgetUsed: number = 0;
  private errorBudgetStart: Date = new Date();
  private reportTimer: NodeJS.Timeout | null = null;
  private reportHistory: SLIReport[] = [];
  private readonly maxReportHistory = 60;

  constructor(config: Partial<SLIManagerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.initializeDefaultSLIs();
  }

  /**
   * Initialize default SSAI SLIs
   */
  private initializeDefaultSLIs(): void {
    // Decision latency SLI
    this.registerSLI({
      name: 'decision_latency_p99',
      description: 'Pod decision latency at p99',
      type: 'latency',
      target: 100, // 100ms
      unit: 'ms',
      windowSeconds: 60,
    });

    // Manifest generation latency
    this.registerSLI({
      name: 'manifest_latency_p99',
      description: 'Manifest generation latency at p99',
      type: 'latency',
      target: 50, // 50ms
      unit: 'ms',
      windowSeconds: 60,
    });

    // Fill rate
    this.registerSLI({
      name: 'fill_rate',
      description: 'Ad slot fill rate',
      type: 'availability',
      target: 95, // 95%
      unit: '%',
      windowSeconds: 300,
    });

    // Error rate
    this.registerSLI({
      name: 'error_rate',
      description: 'Overall error rate',
      type: 'error_rate',
      target: 0.1, // 0.1%
      unit: '%',
      windowSeconds: 60,
    });

    // Tracking success rate
    this.registerSLI({
      name: 'tracking_success_rate',
      description: 'Tracking pixel success rate',
      type: 'availability',
      target: 99, // 99%
      unit: '%',
      windowSeconds: 60,
    });

    // Request throughput
    this.registerSLI({
      name: 'request_throughput',
      description: 'Requests per second',
      type: 'throughput',
      target: 1000, // 1000 rps
      unit: 'rps',
      windowSeconds: 60,
    });
  }

  /**
   * Register an SLI
   */
  registerSLI(definition: SLIDefinition): void {
    this.metrics.set(definition.name, {
      definition,
      dataPoints: [],
      lastValue: 0,
      trend: 'stable',
    });
  }

  /**
   * Record a metric value
   */
  recordMetric(name: string, value: number): void {
    const state = this.metrics.get(name);
    if (!state) return;

    const now = new Date();
    state.dataPoints.push({ timestamp: now, value });
    state.lastValue = value;

    // Trim old data points
    const cutoff = new Date(now.getTime() - this.config.retentionHours * 60 * 60 * 1000);
    state.dataPoints = state.dataPoints.filter((dp) => dp.timestamp > cutoff);

    // Update trend
    state.trend = this.calculateTrend(state.dataPoints, state.definition);

    // Update error budget if applicable
    if (this.config.enableErrorBudget && state.definition.type === 'error_rate') {
      this.updateErrorBudget(value);
    }
  }

  /**
   * Calculate trend from data points
   */
  private calculateTrend(
    dataPoints: MetricDataPoint[],
    definition: SLIDefinition
  ): 'improving' | 'stable' | 'degrading' {
    if (dataPoints.length < 10) return 'stable';

    const recentPoints = dataPoints.slice(-10);
    const oldPoints = dataPoints.slice(-20, -10);

    if (oldPoints.length < 5) return 'stable';

    const recentAvg = recentPoints.reduce((sum, p) => sum + p.value, 0) / recentPoints.length;
    const oldAvg = oldPoints.reduce((sum, p) => sum + p.value, 0) / oldPoints.length;

    const change = ((recentAvg - oldAvg) / oldAvg) * 100;
    const threshold = 5; // 5% change threshold

    // For error rates and latencies, lower is better
    if (definition.type === 'error_rate' || definition.type === 'latency') {
      if (change < -threshold) return 'improving';
      if (change > threshold) return 'degrading';
    } else {
      // For availability and throughput, higher is better
      if (change > threshold) return 'improving';
      if (change < -threshold) return 'degrading';
    }

    return 'stable';
  }

  /**
   * Update error budget
   */
  private updateErrorBudget(errorRate: number): void {
    // Check if we need to reset monthly budget
    const now = new Date();
    const monthDiff =
      (now.getFullYear() - this.errorBudgetStart.getFullYear()) * 12 +
      (now.getMonth() - this.errorBudgetStart.getMonth());

    if (monthDiff >= 1) {
      this.errorBudgetUsed = 0;
      this.errorBudgetStart = now;
    }

    // Accumulate error budget usage
    // This is simplified - real implementation would track per-request errors
    if (errorRate > 0) {
      this.errorBudgetUsed += errorRate / 100;
    }
  }

  /**
   * Get error budget remaining
   */
  getErrorBudgetRemaining(): number {
    const used = this.errorBudgetUsed;
    const budget = this.config.monthlyErrorBudgetPercent;
    return Math.max(0, ((budget - used) / budget) * 100);
  }

  /**
   * Get current metric value
   */
  getMetric(name: string): SLIMetric | null {
    const state = this.metrics.get(name);
    if (!state) return null;

    const meetingTarget = this.isMeetingTarget(state);
    const errorBudgetRemaining =
      state.definition.type === 'error_rate'
        ? this.getErrorBudgetRemaining()
        : 100;

    return {
      name: state.definition.name,
      value: state.lastValue,
      target: state.definition.target,
      meetingTarget,
      errorBudgetRemaining,
      trend: state.trend,
      lastUpdated: state.dataPoints.length > 0
        ? state.dataPoints[state.dataPoints.length - 1].timestamp
        : new Date(),
    };
  }

  /**
   * Check if metric is meeting target
   */
  private isMeetingTarget(state: MetricState): boolean {
    const value = state.lastValue;
    const target = state.definition.target;

    switch (state.definition.type) {
      case 'latency':
      case 'error_rate':
        return value <= target;
      case 'availability':
      case 'throughput':
        return value >= target;
      default:
        return false;
    }
  }

  /**
   * Generate SLI report
   */
  generateReport(): SLIReport {
    const now = new Date();
    const windowSeconds = 300; // 5 minutes
    const windowStart = new Date(now.getTime() - windowSeconds * 1000);

    const metrics: SLIMetric[] = [];
    let unhealthyCount = 0;
    let degradedCount = 0;

    for (const [name] of this.metrics) {
      const metric = this.getMetric(name);
      if (metric) {
        metrics.push(metric);
        if (!metric.meetingTarget) {
          unhealthyCount++;
        } else if (metric.trend === 'degrading') {
          degradedCount++;
        }
      }
    }

    // Determine overall health
    let overallHealth: 'healthy' | 'degraded' | 'unhealthy';
    if (unhealthyCount > 0) {
      overallHealth = 'unhealthy';
    } else if (degradedCount > 0) {
      overallHealth = 'degraded';
    } else {
      overallHealth = 'healthy';
    }

    // Generate recommendations
    const recommendations = this.generateRecommendations(metrics);

    const report: SLIReport = {
      timestamp: now,
      window: {
        start: windowStart,
        end: now,
        seconds: windowSeconds,
      },
      metrics,
      overallHealth,
      recommendations,
    };

    // Store in history
    this.reportHistory.push(report);
    if (this.reportHistory.length > this.maxReportHistory) {
      this.reportHistory.shift();
    }

    return report;
  }

  /**
   * Generate recommendations based on metrics
   */
  private generateRecommendations(metrics: SLIMetric[]): string[] {
    const recommendations: string[] = [];

    for (const metric of metrics) {
      if (!metric.meetingTarget) {
        switch (metric.name) {
          case 'decision_latency_p99':
            recommendations.push(
              'Decision latency is high - consider reducing partner timeout or enabling safe mode'
            );
            break;
          case 'fill_rate':
            recommendations.push(
              'Fill rate is below target - check partner integrations and inventory availability'
            );
            break;
          case 'error_rate':
            recommendations.push(
              'Error rate is elevated - review error logs and consider enabling safe mode'
            );
            break;
          case 'tracking_success_rate':
            recommendations.push(
              'Tracking success rate is low - check tracking URL reachability'
            );
            break;
        }
      }

      if (metric.trend === 'degrading') {
        recommendations.push(
          `${metric.name} is trending downward - monitor closely`
        );
      }

      if (metric.errorBudgetRemaining < 20) {
        recommendations.push(
          `Error budget is low (${metric.errorBudgetRemaining.toFixed(1)}% remaining) - prioritize reliability improvements`
        );
      }
    }

    return recommendations;
  }

  /**
   * Start periodic reporting
   */
  startReporting(): void {
    this.reportTimer = setInterval(() => {
      this.generateReport();
    }, this.config.reportIntervalSeconds * 1000);
  }

  /**
   * Stop periodic reporting
   */
  stopReporting(): void {
    if (this.reportTimer) {
      clearInterval(this.reportTimer);
      this.reportTimer = null;
    }
  }

  /**
   * Get all current metrics
   */
  getAllMetrics(): SLIMetric[] {
    const metrics: SLIMetric[] = [];
    for (const [name] of this.metrics) {
      const metric = this.getMetric(name);
      if (metric) metrics.push(metric);
    }
    return metrics;
  }

  /**
   * Get report history
   */
  getReportHistory(): SLIReport[] {
    return [...this.reportHistory];
  }

  /**
   * Get latest report
   */
  getLatestReport(): SLIReport | null {
    if (this.reportHistory.length === 0) {
      return this.generateReport();
    }
    return this.reportHistory[this.reportHistory.length - 1];
  }

  /**
   * Clear all state
   */
  clear(): void {
    for (const state of this.metrics.values()) {
      state.dataPoints = [];
      state.lastValue = 0;
      state.trend = 'stable';
    }
    this.errorBudgetUsed = 0;
    this.errorBudgetStart = new Date();
    this.reportHistory = [];
  }
}
