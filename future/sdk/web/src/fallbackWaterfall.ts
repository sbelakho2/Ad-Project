/**
 * FallbackWaterfall - Implements a waterfall mediation strategy for ad loading.
 *
 * When one ad source fails, it automatically falls back to the next source
 * in the waterfall. This ensures maximum fill rate by trying multiple
 * demand partners in priority order.
 */

/** Default timeout for source loading (ms) */
export const DEFAULT_TIMEOUT_MS = 5000;

/** Result type for attempts */
export type AttemptResultType = 'success' | 'noFill' | 'error' | 'timeout' | 'skipped';

/** Result of attempting to load from a source */
export type WaterfallResult<T> =
  | { type: 'success'; data: T }
  | { type: 'noFill'; reason: string }
  | { type: 'error'; error: Error }
  | { type: 'timeout' };

/** Represents an ad source in the waterfall */
export interface WaterfallSource<T> {
  readonly id: string;
  readonly priority: number;
  readonly timeoutMs: number;
  readonly loader: (signal: AbortSignal) => Promise<WaterfallResult<T>>;
}

/** Details about each attempt in the waterfall */
export interface AttemptDetail {
  readonly sourceId: string;
  readonly priority: number;
  readonly durationMs: number;
  readonly result: AttemptResultType;
}

/** Waterfall execution result with metadata */
export interface ExecutionResult<T> {
  readonly result: WaterfallResult<T>;
  readonly sourceId: string;
  readonly attemptsCount: number;
  readonly totalDurationMs: number;
  readonly attemptDetails: readonly AttemptDetail[];
}

/** Performance statistics for a source */
export interface SourceStats {
  successCount: number;
  failureCount: number;
  timeoutCount: number;
  noFillCount: number;
  totalLatencyMs: number;
  readonly totalAttempts: number;
  readonly successRate: number;
  readonly averageLatencyMs: number;
}

function createSourceStats(): SourceStats {
  return {
    successCount: 0,
    failureCount: 0,
    timeoutCount: 0,
    noFillCount: 0,
    totalLatencyMs: 0,
    get totalAttempts() {
      return this.successCount + this.failureCount + this.timeoutCount + this.noFillCount;
    },
    get successRate() {
      return this.totalAttempts > 0 ? this.successCount / this.totalAttempts : 0;
    },
    get averageLatencyMs() {
      return this.totalAttempts > 0 ? this.totalLatencyMs / this.totalAttempts : 0;
    }
  };
}

/** FallbackWaterfall class for managing waterfall execution */
export class FallbackWaterfall<T> {
  private readonly defaultTimeoutMs: number;
  private readonly sourceStats = new Map<string, SourceStats>();
  
  constructor(defaultTimeoutMs: number = DEFAULT_TIMEOUT_MS) {
    this.defaultTimeoutMs = defaultTimeoutMs;
  }
  
  /**
   * Executes the waterfall, trying each source in priority order until
   * one succeeds or all sources are exhausted.
   */
  async execute(
    sources: readonly WaterfallSource<T>[],
    signal?: AbortSignal
  ): Promise<ExecutionResult<T>> {
    const startTime = Date.now();
    const attemptDetails: AttemptDetail[] = [];
    const sortedSources = [...sources].sort((a, b) => a.priority - b.priority);
    
    for (const source of sortedSources) {
      // Check for cancellation
      if (signal?.aborted) {
        throw new DOMException('Waterfall execution aborted', 'AbortError');
      }
      
      const attemptStart = Date.now();
      let result: WaterfallResult<T>;
      
      try {
        result = await this.executeWithTimeout(source, signal);
      } catch (error) {
        if (error instanceof DOMException && error.name === 'AbortError') {
          result = { type: 'timeout' };
        } else {
          result = { type: 'error', error: error instanceof Error ? error : new Error(String(error)) };
        }
      }
      
      const durationMs = Date.now() - attemptStart;
      this.recordStats(source.id, result, durationMs);
      
      const attemptResultType = this.resultToType(result);
      attemptDetails.push({
        sourceId: source.id,
        priority: source.priority,
        durationMs,
        result: attemptResultType
      });
      
      if (result.type === 'success') {
        return {
          result,
          sourceId: source.id,
          attemptsCount: attemptDetails.length,
          totalDurationMs: Date.now() - startTime,
          attemptDetails
        };
      }
    }
    
    // All sources exhausted
    return {
      result: { type: 'noFill', reason: `All ${sortedSources.length} sources exhausted` },
      sourceId: sortedSources[sortedSources.length - 1]?.id ?? 'none',
      attemptsCount: attemptDetails.length,
      totalDurationMs: Date.now() - startTime,
      attemptDetails
    };
  }
  
  /**
   * Executes waterfall with parallel preloading of lower-priority sources.
   */
  async executeWithParallelPreload(
    sources: readonly WaterfallSource<T>[],
    preloadCount: number = 1,
    signal?: AbortSignal
  ): Promise<ExecutionResult<T>> {
    const startTime = Date.now();
    const attemptDetails: AttemptDetail[] = [];
    const sortedSources = [...sources].sort((a, b) => a.priority - b.priority);
    
    const preloadResults = new Map<string, Promise<WaterfallResult<T>>>();
    
    for (let i = 0; i < sortedSources.length; i++) {
      const source = sortedSources[i];
      
      if (signal?.aborted) {
        throw new DOMException('Waterfall execution aborted', 'AbortError');
      }
      
      // Start preloading next sources
      for (let j = i + 1; j <= i + preloadCount && j < sortedSources.length; j++) {
        const preloadSource = sortedSources[j];
        if (!preloadResults.has(preloadSource.id)) {
          preloadResults.set(
            preloadSource.id,
            this.executeWithTimeout(preloadSource, signal).catch(err => ({
              type: 'error' as const,
              error: err instanceof Error ? err : new Error(String(err))
            }))
          );
        }
      }
      
      const attemptStart = Date.now();
      let result: WaterfallResult<T>;
      
      // Use preloaded result if available, otherwise execute now
      const preloadedPromise = preloadResults.get(source.id);
      if (preloadedPromise) {
        try {
          result = await preloadedPromise;
        } catch (error) {
          result = { type: 'error', error: error instanceof Error ? error : new Error(String(error)) };
        }
      } else {
        try {
          result = await this.executeWithTimeout(source, signal);
        } catch (error) {
          if (error instanceof DOMException && error.name === 'AbortError') {
            result = { type: 'timeout' };
          } else {
            result = { type: 'error', error: error instanceof Error ? error : new Error(String(error)) };
          }
        }
      }
      
      const durationMs = Date.now() - attemptStart;
      this.recordStats(source.id, result, durationMs);
      
      attemptDetails.push({
        sourceId: source.id,
        priority: source.priority,
        durationMs,
        result: this.resultToType(result)
      });
      
      if (result.type === 'success') {
        return {
          result,
          sourceId: source.id,
          attemptsCount: attemptDetails.length,
          totalDurationMs: Date.now() - startTime,
          attemptDetails
        };
      }
    }
    
    return {
      result: { type: 'noFill', reason: `All ${sortedSources.length} sources exhausted` },
      sourceId: sortedSources[sortedSources.length - 1]?.id ?? 'none',
      attemptsCount: attemptDetails.length,
      totalDurationMs: Date.now() - startTime,
      attemptDetails
    };
  }
  
  /**
   * Gets statistics for a specific source.
   */
  getStats(sourceId: string): SourceStats | undefined {
    return this.sourceStats.get(sourceId);
  }
  
  /**
   * Gets statistics for all sources.
   */
  getAllStats(): Map<string, SourceStats> {
    return new Map(this.sourceStats);
  }
  
  /**
   * Clears all recorded statistics.
   */
  clearStats(): void {
    this.sourceStats.clear();
  }
  
  /**
   * Creates a source with the given parameters.
   */
  createSource(
    id: string,
    priority: number,
    loader: (signal: AbortSignal) => Promise<WaterfallResult<T>>,
    timeoutMs: number = this.defaultTimeoutMs
  ): WaterfallSource<T> {
    return { id, priority, timeoutMs, loader };
  }
  
  private async executeWithTimeout(
    source: WaterfallSource<T>,
    parentSignal?: AbortSignal
  ): Promise<WaterfallResult<T>> {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), source.timeoutMs);
    
    // Link to parent signal
    const abortHandler = () => controller.abort();
    parentSignal?.addEventListener('abort', abortHandler);
    
    try {
      return await source.loader(controller.signal);
    } finally {
      clearTimeout(timeoutId);
      parentSignal?.removeEventListener('abort', abortHandler);
    }
  }
  
  private resultToType(result: WaterfallResult<T>): AttemptResultType {
    switch (result.type) {
      case 'success': return 'success';
      case 'noFill': return 'noFill';
      case 'error': return 'error';
      case 'timeout': return 'timeout';
    }
  }
  
  private recordStats(sourceId: string, result: WaterfallResult<T>, durationMs: number): void {
    let stats = this.sourceStats.get(sourceId);
    if (!stats) {
      stats = createSourceStats();
      this.sourceStats.set(sourceId, stats);
    }
    
    stats.totalLatencyMs += durationMs;
    
    switch (result.type) {
      case 'success':
        stats.successCount++;
        break;
      case 'noFill':
        stats.noFillCount++;
        break;
      case 'error':
        stats.failureCount++;
        break;
      case 'timeout':
        stats.timeoutCount++;
        break;
    }
  }
}

/** Creates a new FallbackWaterfall instance */
export function createWaterfall<T>(defaultTimeoutMs?: number): FallbackWaterfall<T> {
  return new FallbackWaterfall<T>(defaultTimeoutMs);
}

/** Helper to create a success result */
export function success<T>(data: T): WaterfallResult<T> {
  return { type: 'success', data };
}

/** Helper to create a no-fill result */
export function noFill<T>(reason: string = 'No ad available'): WaterfallResult<T> {
  return { type: 'noFill', reason };
}

/** Helper to create an error result */
export function error<T>(err: Error): WaterfallResult<T> {
  return { type: 'error', error: err };
}

/** Helper to create a timeout result */
export function timeout<T>(): WaterfallResult<T> {
  return { type: 'timeout' };
}
