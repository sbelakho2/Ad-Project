/**
 * PriorityWeightedMediation - Intelligent ad source selection
 *
 * Implements intelligent ad source selection based on:
 * - Configured priority (lower = higher priority)
 * - Weighted random selection within same priority tier
 * - Performance-based weight adjustments
 * - Fill rate optimization
 */

/**
 * Ad source with priority and weight configuration
 */
export interface WeightedAdSource {
  id: string;
  priority: number;
  weight: number;
  timeout: number;
  enabled: boolean;
  minBid: number;
  metadata: Record<string, string>;
}

/**
 * Result of loading an ad from a source
 */
export type AdLoadResult =
  | { type: 'success'; ad: unknown; sourceId: string; latencyMs: number; bid: number }
  | { type: 'noFill'; sourceId: string; reason: string }
  | { type: 'error'; sourceId: string; error: Error }
  | { type: 'timeout'; sourceId: string };

/**
 * Statistics for a source
 */
export interface SourcePerformance {
  sourceId: string;
  totalAttempts: number;
  successCount: number;
  noFillCount: number;
  errorCount: number;
  timeoutCount: number;
  averageLatencyMs: number;
  fillRate: number;
  averageBid: number;
  effectiveWeight: number;
}

/**
 * Configuration for priority-weighted mediation
 */
export interface PriorityWeightedConfig {
  usePerformanceWeighting: boolean;
  performanceWindowMs: number;
  minSampleSize: number;
  weightDecayFactor: number;
  maxConcurrentRequests: number;
  bidFloorEnabled: boolean;
  adaptiveTimeoutsEnabled: boolean;
}

/**
 * Default configuration
 */
export const DEFAULT_CONFIG: PriorityWeightedConfig = {
  usePerformanceWeighting: true,
  performanceWindowMs: 3600000, // 1 hour
  minSampleSize: 10,
  weightDecayFactor: 0.1,
  maxConcurrentRequests: 3,
  bidFloorEnabled: true,
  adaptiveTimeoutsEnabled: true,
};

/**
 * Ad loader function type
 */
export type AdLoader = (
  source: WeightedAdSource,
  signal?: AbortSignal
) => Promise<AdLoadResult>;

/**
 * Internal source statistics
 */
interface SourceStats {
  attempts: number;
  successes: number;
  noFills: number;
  errors: number;
  timeouts: number;
  totalLatencyMs: number;
  totalBidMicros: number;
  lastSuccessTime: number;
}

/**
 * Priority-weighted mediation manager
 */
export class PriorityWeightedMediation {
  private config: PriorityWeightedConfig;
  private sourceStats: Map<string, SourceStats> = new Map();

  constructor(config: Partial<PriorityWeightedConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Execute mediation with priority-weighted selection
   */
  async execute(
    sources: WeightedAdSource[],
    loader: AdLoader,
    abortSignal?: AbortSignal
  ): Promise<AdLoadResult> {
    if (sources.length === 0) {
      return { type: 'noFill', sourceId: 'none', reason: 'No sources configured' };
    }

    const enabledSources = sources.filter((s) => s.enabled);
    if (enabledSources.length === 0) {
      return { type: 'noFill', sourceId: 'none', reason: 'All sources disabled' };
    }

    // Group by priority
    const priorityGroups = this.groupByPriority(enabledSources);
    const sortedPriorities = [...priorityGroups.keys()].sort((a, b) => a - b);

    let lastResult: AdLoadResult | null = null;

    // Try each priority group in order
    for (const priority of sortedPriorities) {
      if (abortSignal?.aborted) {
        return { type: 'noFill', sourceId: 'none', reason: 'Aborted' };
      }

      const group = priorityGroups.get(priority)!;
      const result = await this.executePriorityGroup(group, loader, abortSignal);
      lastResult = result;

      if (result.type === 'success') {
        return result;
      }
    }

    // Return the last result (timeout or error) if available, otherwise noFill
    if (lastResult && (lastResult.type === 'timeout' || lastResult.type === 'error')) {
      return lastResult;
    }

    return { type: 'noFill', sourceId: 'all', reason: 'All sources exhausted' };
  }

  /**
   * Get performance statistics for all sources
   */
  getPerformanceStats(): SourcePerformance[] {
    const stats: SourcePerformance[] = [];

    for (const [sourceId, source] of this.sourceStats) {
      const avgLatency =
        source.successes > 0 ? source.totalLatencyMs / source.successes : 0;
      const fillRate =
        source.attempts > 0 ? source.successes / source.attempts : 0;
      const avgBid =
        source.successes > 0
          ? source.totalBidMicros / (source.successes * 1_000_000)
          : 0;

      stats.push({
        sourceId,
        totalAttempts: source.attempts,
        successCount: source.successes,
        noFillCount: source.noFills,
        errorCount: source.errors,
        timeoutCount: source.timeouts,
        averageLatencyMs: avgLatency,
        fillRate,
        averageBid: avgBid,
        effectiveWeight: 1.0, // Would need source reference for actual calculation
      });
    }

    return stats;
  }

  /**
   * Get performance for a specific source
   */
  getSourcePerformance(sourceId: string): SourcePerformance | null {
    const source = this.sourceStats.get(sourceId);
    if (!source) return null;

    const avgLatency =
      source.successes > 0 ? source.totalLatencyMs / source.successes : 0;
    const fillRate =
      source.attempts > 0 ? source.successes / source.attempts : 0;
    const avgBid =
      source.successes > 0
        ? source.totalBidMicros / (source.successes * 1_000_000)
        : 0;

    return {
      sourceId,
      totalAttempts: source.attempts,
      successCount: source.successes,
      noFillCount: source.noFills,
      errorCount: source.errors,
      timeoutCount: source.timeouts,
      averageLatencyMs: avgLatency,
      fillRate,
      averageBid: avgBid,
      effectiveWeight: 1.0,
    };
  }

  /**
   * Reset statistics for a source
   */
  resetStats(sourceId: string): void {
    this.sourceStats.delete(sourceId);
  }

  /**
   * Reset all statistics
   */
  resetAllStats(): void {
    this.sourceStats.clear();
  }

  /**
   * Update configuration
   */
  updateConfiguration(config: Partial<PriorityWeightedConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration
   */
  getConfiguration(): PriorityWeightedConfig {
    return { ...this.config };
  }

  private groupByPriority(
    sources: WeightedAdSource[]
  ): Map<number, WeightedAdSource[]> {
    const groups = new Map<number, WeightedAdSource[]>();

    for (const source of sources) {
      const existing = groups.get(source.priority) || [];
      existing.push(source);
      groups.set(source.priority, existing);
    }

    return groups;
  }

  private async executePriorityGroup(
    sources: WeightedAdSource[],
    loader: AdLoader,
    abortSignal?: AbortSignal
  ): Promise<AdLoadResult> {
    const remainingSources = [...sources];
    let lastResult: AdLoadResult | null = null;

    while (remainingSources.length > 0) {
      if (abortSignal?.aborted) {
        return { type: 'noFill', sourceId: 'none', reason: 'Aborted' };
      }

      const selected = this.selectByWeight(remainingSources);
      const index = remainingSources.findIndex((s) => s.id === selected.id);
      remainingSources.splice(index, 1);

      const result = await this.executeWithTimeout(selected, loader, abortSignal);
      this.recordResult(selected.id, result);
      lastResult = result;

      if (result.type === 'success') {
        return result;
      }
    }

    // If the last result was a timeout, return it instead of noFill
    if (lastResult?.type === 'timeout') {
      return lastResult;
    }

    return {
      type: 'noFill',
      sourceId: 'priority_group',
      reason: 'No fill from priority group',
    };
  }

  private selectByWeight(sources: WeightedAdSource[]): WeightedAdSource {
    if (sources.length === 1) {
      return sources[0];
    }

    const effectiveWeights = sources.map((source) => ({
      source,
      weight: this.calculateEffectiveWeight(source),
    }));

    const totalWeight = effectiveWeights.reduce((sum, e) => sum + e.weight, 0);

    if (totalWeight <= 0) {
      return sources[Math.floor(Math.random() * sources.length)];
    }

    let random = Math.random() * totalWeight;

    for (const { source, weight } of effectiveWeights) {
      random -= weight;
      if (random <= 0) {
        return source;
      }
    }

    return sources[sources.length - 1];
  }

  private calculateEffectiveWeight(source: WeightedAdSource): number {
    const baseWeight = source.weight;

    if (!this.config.usePerformanceWeighting) {
      return baseWeight;
    }

    const stats = this.sourceStats.get(source.id);
    if (!stats || stats.attempts < this.config.minSampleSize) {
      return baseWeight;
    }

    const fillRate = stats.successes / Math.max(1, stats.attempts);
    const performanceMultiplier = 0.5 + fillRate;

    return Math.max(0.1, baseWeight * performanceMultiplier);
  }

  private async executeWithTimeout(
    source: WeightedAdSource,
    loader: AdLoader,
    parentSignal?: AbortSignal
  ): Promise<AdLoadResult> {
    const timeout = this.config.adaptiveTimeoutsEnabled
      ? this.calculateAdaptiveTimeout(source)
      : source.timeout;

    const controller = new AbortController();
    const signal = controller.signal;

    // Link to parent signal
    if (parentSignal) {
      if (parentSignal.aborted) {
        return { type: 'timeout', sourceId: source.id };
      }
      parentSignal.addEventListener('abort', () => controller.abort());
    }

    // Timeout promise that rejects after the timeout period
    let timeoutId: ReturnType<typeof setTimeout>;
    const timeoutPromise = new Promise<AdLoadResult>((_resolve, reject) => {
      timeoutId = setTimeout(() => {
        controller.abort();
        reject(new Error('TIMEOUT'));
      }, timeout);
    });

    // Abort promise that rejects when signal is aborted
    const abortPromise = new Promise<AdLoadResult>((_resolve, reject) => {
      signal.addEventListener('abort', () => {
        reject(new Error('ABORTED'));
      });
    });

    try {
      const result = await Promise.race([
        loader(source, signal),
        timeoutPromise,
        abortPromise
      ]);
      clearTimeout(timeoutId!);
      return result;
    } catch (error) {
      clearTimeout(timeoutId!);
      if (signal.aborted || (error instanceof Error && (error.message === 'TIMEOUT' || error.message === 'ABORTED'))) {
        return { type: 'timeout', sourceId: source.id };
      }

      return {
        type: 'error',
        sourceId: source.id,
        error: error instanceof Error ? error : new Error(String(error)),
      };
    }
  }

  private calculateAdaptiveTimeout(source: WeightedAdSource): number {
    const stats = this.sourceStats.get(source.id);

    if (!stats || stats.successes < this.config.minSampleSize) {
      return source.timeout;
    }

    const avgLatencyMs = stats.totalLatencyMs / stats.successes;
    const adaptiveTimeout = avgLatencyMs * 2;

    return Math.min(source.timeout, Math.max(1000, adaptiveTimeout));
  }

  private recordResult(sourceId: string, result: AdLoadResult): void {
    let stats = this.sourceStats.get(sourceId);

    if (!stats) {
      stats = {
        attempts: 0,
        successes: 0,
        noFills: 0,
        errors: 0,
        timeouts: 0,
        totalLatencyMs: 0,
        totalBidMicros: 0,
        lastSuccessTime: 0,
      };
      this.sourceStats.set(sourceId, stats);
    }

    stats.attempts++;

    switch (result.type) {
      case 'success':
        stats.successes++;
        stats.totalLatencyMs += result.latencyMs;
        stats.totalBidMicros += result.bid * 1_000_000;
        stats.lastSuccessTime = Date.now();
        break;
      case 'noFill':
        stats.noFills++;
        break;
      case 'error':
        stats.errors++;
        break;
      case 'timeout':
        stats.timeouts++;
        break;
    }
  }
}

/**
 * Builder for WeightedAdSource
 */
export class WeightedAdSourceBuilder {
  private source: Partial<WeightedAdSource> = {
    priority: 0,
    weight: 1.0,
    timeout: 5000,
    enabled: true,
    minBid: 0,
    metadata: {},
  };

  id(id: string): this {
    this.source.id = id;
    return this;
  }

  priority(priority: number): this {
    this.source.priority = priority;
    return this;
  }

  weight(weight: number): this {
    this.source.weight = weight;
    return this;
  }

  timeout(timeout: number): this {
    this.source.timeout = timeout;
    return this;
  }

  enabled(enabled: boolean): this {
    this.source.enabled = enabled;
    return this;
  }

  minBid(minBid: number): this {
    this.source.minBid = minBid;
    return this;
  }

  metadata(key: string, value: string): this {
    this.source.metadata = this.source.metadata || {};
    this.source.metadata[key] = value;
    return this;
  }

  build(): WeightedAdSource {
    if (!this.source.id) {
      throw new Error('Source ID is required');
    }
    if (this.source.priority! < 0) {
      throw new Error('Priority must be non-negative');
    }
    if (this.source.weight! <= 0) {
      throw new Error('Weight must be positive');
    }

    return {
      id: this.source.id,
      priority: this.source.priority!,
      weight: this.source.weight!,
      timeout: this.source.timeout!,
      enabled: this.source.enabled!,
      minBid: this.source.minBid!,
      metadata: this.source.metadata!,
    };
  }
}

/**
 * Helper function to create a weighted ad source
 */
export function createWeightedAdSource(
  id: string,
  priority: number,
  options: Partial<Omit<WeightedAdSource, 'id' | 'priority'>> = {}
): WeightedAdSource {
  return {
    id,
    priority,
    weight: options.weight ?? 1.0,
    timeout: options.timeout ?? 5000,
    enabled: options.enabled ?? true,
    minBid: options.minBid ?? 0,
    metadata: options.metadata ?? {},
  };
}
