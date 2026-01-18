/**
 * SSAI Metrics
 * Collects and exports metrics for SSAI operations
 */

export interface HistogramStats {
  count: number;
  sum: number;
  avg: number;
  min: number;
  max: number;
}

export class SSAIMetrics {
  private counters: Map<string, Map<string, number>>;
  private gauges: Map<string, number>;
  private histograms: Map<string, number[]>;
  private fillRates: Map<string, { filled: number; requested: number }>;
  private decisionLatencies: Map<string, number[]>;

  constructor() {
    this.counters = new Map();
    this.gauges = new Map();
    this.histograms = new Map();
    this.fillRates = new Map();
    this.decisionLatencies = new Map();
  }

  /**
   * Increment a counter
   */
  incrementCounter(name: string, labels: Record<string, string> = {}): void {
    const labelKey = this.labelsToKey(labels);
    
    if (!this.counters.has(name)) {
      this.counters.set(name, new Map());
    }
    
    const counter = this.counters.get(name)!;
    const current = counter.get(labelKey) || 0;
    counter.set(labelKey, current + 1);
  }

  /**
   * Get counter value
   */
  getCounter(name: string, labels: Record<string, string> = {}): number {
    const labelKey = this.labelsToKey(labels);
    return this.counters.get(name)?.get(labelKey) || 0;
  }

  /**
   * Record histogram value
   */
  recordHistogram(name: string, value: number): void {
    if (!this.histograms.has(name)) {
      this.histograms.set(name, []);
    }
    this.histograms.get(name)!.push(value);
  }

  /**
   * Get histogram statistics
   */
  getHistogramStats(name: string): HistogramStats {
    const values = this.histograms.get(name) || [];
    
    if (values.length === 0) {
      return { count: 0, sum: 0, avg: 0, min: 0, max: 0 };
    }

    const sum = values.reduce((a, b) => a + b, 0);
    
    return {
      count: values.length,
      sum,
      avg: sum / values.length,
      min: Math.min(...values),
      max: Math.max(...values),
    };
  }

  /**
   * Set gauge value
   */
  setGauge(name: string, value: number): void {
    this.gauges.set(name, value);
  }

  /**
   * Get gauge value
   */
  getGauge(name: string): number {
    return this.gauges.get(name) || 0;
  }

  /**
   * Record fill rate
   */
  recordFillRate(partnerId: string, data: { filled: number; requested: number }): void {
    this.fillRates.set(partnerId, data);
  }

  /**
   * Get fill rate
   */
  getFillRate(partnerId: string): number {
    const data = this.fillRates.get(partnerId);
    if (!data || data.requested === 0) return 0;
    return data.filled / data.requested;
  }

  /**
   * Get fill rate entries
   */
  getFillRateEntries(): Array<{ partnerId: string; filled: number; requested: number; fillRate: number }> {
    const entries: Array<{ partnerId: string; filled: number; requested: number; fillRate: number }> = [];
    for (const [partnerId, data] of this.fillRates.entries()) {
      const requested = data.requested || 0;
      const filled = data.filled || 0;
      const fillRate = requested > 0 ? filled / requested : 0;
      entries.push({ partnerId, filled, requested, fillRate });
    }
    return entries;
  }

  /**
   * Record decision latency
   */
  recordDecisionLatency(partnerId: string, latencyMs: number): void {
    if (!this.decisionLatencies.has(partnerId)) {
      this.decisionLatencies.set(partnerId, []);
    }
    this.decisionLatencies.get(partnerId)!.push(latencyMs);
  }

  /**
   * Get decision latency P99
   */
  getDecisionLatencyP99(partnerId: string): number {
    const latencies = this.decisionLatencies.get(partnerId) || [];
    if (latencies.length === 0) return 0;

    const sorted = [...latencies].sort((a, b) => a - b);
    const index = Math.ceil(sorted.length * 0.99) - 1;
    return sorted[index];
  }

  /**
   * Get decision latency entries
   */
  getDecisionLatencyEntries(): Array<{ partnerId: string; p99Ms: number; count: number }> {
    const entries: Array<{ partnerId: string; p99Ms: number; count: number }> = [];
    for (const [partnerId, latencies] of this.decisionLatencies.entries()) {
      const count = latencies.length;
      const p99Ms = count > 0 ? this.getDecisionLatencyP99(partnerId) : 0;
      entries.push({ partnerId, p99Ms, count });
    }
    return entries;
  }

  /**
   * Export metrics in Prometheus format
   */
  exportPrometheus(): string {
    const lines: string[] = [];

    // Export counters
    for (const [name, labelMap] of this.counters) {
      lines.push(`# HELP ${name} Counter metric`);
      lines.push(`# TYPE ${name} counter`);
      for (const [labels, value] of labelMap) {
        if (labels) {
          lines.push(`${name}{${labels}} ${value}`);
        } else {
          lines.push(`${name} ${value}`);
        }
      }
    }

    // Export gauges
    for (const [name, value] of this.gauges) {
      lines.push(`# HELP ${name} Gauge metric`);
      lines.push(`# TYPE ${name} gauge`);
      lines.push(`${name} ${value}`);
    }

    // Export histograms as summary
    for (const [name, values] of this.histograms) {
      const stats = this.getHistogramStats(name);
      lines.push(`# HELP ${name} Histogram metric`);
      lines.push(`# TYPE ${name} summary`);
      lines.push(`${name}_count ${stats.count}`);
      lines.push(`${name}_sum ${stats.sum}`);
    }

    return lines.join('\n');
  }

  /**
   * Reset all metrics
   */
  reset(): void {
    this.counters.clear();
    this.gauges.clear();
    this.histograms.clear();
    this.fillRates.clear();
    this.decisionLatencies.clear();
  }

  private labelsToKey(labels: Record<string, string>): string {
    const pairs = Object.entries(labels)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([k, v]) => `${k}="${v}"`);
    return pairs.join(',');
  }
}

export default SSAIMetrics;
