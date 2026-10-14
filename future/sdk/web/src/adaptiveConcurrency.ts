/**
 * AdaptiveConcurrency - Adaptive task scheduling based on device capabilities.
 * 
 * This class provides adaptive concurrency management for web that:
 * - Detects hardware concurrency (logical CPU cores)
 * - Limits concurrent operations appropriately
 * - Provides separate queues for different operation types
 * - Uses requestIdleCallback for low-priority work
 */

export interface AdaptiveConcurrencyConfig {
  /** Override for max background tasks */
  maxBackgroundTasks?: number;
  /** Override for max network tasks */
  maxNetworkTasks?: number;
  /** Override for max compute tasks */
  maxComputeTasks?: number;
  /** Enable debug logging */
  debug?: boolean;
}

export interface DeviceInfo {
  /** Number of logical CPU cores */
  hardwareConcurrency: number;
  /** Device memory in GB (if available) */
  deviceMemory: number | null;
  /** Whether this appears to be a low-end device */
  isLowEndDevice: boolean;
  /** User agent string */
  userAgent: string;
  /** Whether running in a worker context */
  isWorker: boolean;
}

export interface PoolConfig {
  maxConcurrency: number;
}

export interface ConcurrencyStats {
  deviceInfo: DeviceInfo;
  backgroundQueue: { pending: number; running: number; maxConcurrency: number };
  networkQueue: { pending: number; running: number; maxConcurrency: number };
  computeQueue: { pending: number; running: number; maxConcurrency: number };
  totalTasksSubmitted: number;
  totalTasksCompleted: number;
}

interface QueuedTask<T> {
  task: () => Promise<T>;
  resolve: (value: T) => void;
  reject: (error: Error) => void;
}

// Constants
const MIN_CONCURRENT_TASKS = 2;
const MAX_BACKGROUND_TASKS = 8;
const MAX_NETWORK_TASKS = 16;
const MAX_COMPUTE_TASKS = 4;

/**
 * Task queue with concurrency limiting.
 */
class TaskQueue {
  private pending: QueuedTask<unknown>[] = [];
  private running = 0;
  private maxConcurrency: number;

  constructor(maxConcurrency: number) {
    this.maxConcurrency = maxConcurrency;
  }

  async execute<T>(task: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      this.pending.push({ task, resolve: resolve as (v: unknown) => void, reject });
      this.processQueue();
    });
  }

  private async processQueue(): Promise<void> {
    if (this.running >= this.maxConcurrency || this.pending.length === 0) {
      return;
    }

    const queued = this.pending.shift();
    if (!queued) return;

    this.running++;

    try {
      const result = await queued.task();
      queued.resolve(result);
    } catch (error) {
      queued.reject(error instanceof Error ? error : new Error(String(error)));
    } finally {
      this.running--;
      this.processQueue();
    }
  }

  getStats(): { pending: number; running: number; maxConcurrency: number } {
    return {
      pending: this.pending.length,
      running: this.running,
      maxConcurrency: this.maxConcurrency,
    };
  }

  updateMaxConcurrency(value: number): void {
    this.maxConcurrency = value;
    this.processQueue();
  }
}

/**
 * AdaptiveConcurrency manages task execution with adaptive limits based on device capabilities.
 */
export class AdaptiveConcurrency {
  private static instance: AdaptiveConcurrency | null = null;

  private readonly deviceInfo: DeviceInfo;
  private readonly config: Required<AdaptiveConcurrencyConfig>;
  
  private readonly backgroundQueue: TaskQueue;
  private readonly networkQueue: TaskQueue;
  private readonly computeQueue: TaskQueue;
  
  private taskCount = 0;
  private completedCount = 0;

  private constructor(config: AdaptiveConcurrencyConfig = {}) {
    this.deviceInfo = AdaptiveConcurrency.detectDeviceInfo();
    
    this.config = {
      maxBackgroundTasks: config.maxBackgroundTasks ?? this.calculateBackgroundLimit(),
      maxNetworkTasks: config.maxNetworkTasks ?? this.calculateNetworkLimit(),
      maxComputeTasks: config.maxComputeTasks ?? this.calculateComputeLimit(),
      debug: config.debug ?? false,
    };

    this.backgroundQueue = new TaskQueue(this.config.maxBackgroundTasks);
    this.networkQueue = new TaskQueue(this.config.maxNetworkTasks);
    this.computeQueue = new TaskQueue(this.config.maxComputeTasks);

    this.log(`Initialized: ${this.deviceInfo.hardwareConcurrency} cores, ` +
      `lowEnd: ${this.deviceInfo.isLowEndDevice}, ` +
      `limits: bg=${this.config.maxBackgroundTasks}, net=${this.config.maxNetworkTasks}, cpu=${this.config.maxComputeTasks}`);
  }

  /**
   * Gets the singleton instance of AdaptiveConcurrency.
   */
  static getInstance(config?: AdaptiveConcurrencyConfig): AdaptiveConcurrency {
    if (!AdaptiveConcurrency.instance) {
      AdaptiveConcurrency.instance = new AdaptiveConcurrency(config);
    }
    return AdaptiveConcurrency.instance;
  }

  /**
   * Resets the singleton instance.
   */
  static reset(): void {
    AdaptiveConcurrency.instance = null;
  }

  /**
   * Gets detected device information.
   */
  getDeviceInfo(): DeviceInfo {
    return { ...this.deviceInfo };
  }

  /**
   * Gets the recommended pool configuration for background work.
   */
  getBackgroundPoolConfig(): PoolConfig {
    return { maxConcurrency: this.config.maxBackgroundTasks };
  }

  /**
   * Gets the recommended pool configuration for network I/O.
   */
  getNetworkPoolConfig(): PoolConfig {
    return { maxConcurrency: this.config.maxNetworkTasks };
  }

  /**
   * Gets the recommended pool configuration for compute-intensive work.
   */
  getComputePoolConfig(): PoolConfig {
    return { maxConcurrency: this.config.maxComputeTasks };
  }

  /**
   * Executes a background task with concurrency limiting.
   */
  async executeBackground<T>(task: () => Promise<T>): Promise<T> {
    this.taskCount++;
    try {
      const result = await this.backgroundQueue.execute(task);
      this.completedCount++;
      return result;
    } catch (error) {
      this.completedCount++;
      throw error;
    }
  }

  /**
   * Executes a network I/O task with concurrency limiting.
   */
  async executeNetwork<T>(task: () => Promise<T>): Promise<T> {
    this.taskCount++;
    try {
      const result = await this.networkQueue.execute(task);
      this.completedCount++;
      return result;
    } catch (error) {
      this.completedCount++;
      throw error;
    }
  }

  /**
   * Executes a compute-intensive task with concurrency limiting.
   */
  async executeCompute<T>(task: () => Promise<T>): Promise<T> {
    this.taskCount++;
    try {
      const result = await this.computeQueue.execute(task);
      this.completedCount++;
      return result;
    } catch (error) {
      this.completedCount++;
      throw error;
    }
  }

  /**
   * Executes a task during browser idle time.
   * Falls back to setTimeout if requestIdleCallback is not available.
   */
  executeWhenIdle<T>(task: () => T, timeout = 5000): Promise<T> {
    return new Promise((resolve, reject) => {
      const callback = () => {
        try {
          resolve(task());
        } catch (error) {
          reject(error);
        }
      };

      if (typeof requestIdleCallback !== 'undefined') {
        requestIdleCallback(callback, { timeout });
      } else {
        setTimeout(callback, 0);
      }
    });
  }

  /**
   * Executes multiple tasks with concurrency limiting.
   */
  async executeAll<T>(
    tasks: (() => Promise<T>)[],
    type: 'background' | 'network' | 'compute' = 'background'
  ): Promise<T[]> {
    const executeMethod = {
      background: (t: () => Promise<T>) => this.executeBackground(t),
      network: (t: () => Promise<T>) => this.executeNetwork(t),
      compute: (t: () => Promise<T>) => this.executeCompute(t),
    }[type];

    return Promise.all(tasks.map(executeMethod));
  }

  /**
   * Gets current concurrency statistics.
   */
  getStats(): ConcurrencyStats {
    return {
      deviceInfo: { ...this.deviceInfo },
      backgroundQueue: this.backgroundQueue.getStats(),
      networkQueue: this.networkQueue.getStats(),
      computeQueue: this.computeQueue.getStats(),
      totalTasksSubmitted: this.taskCount,
      totalTasksCompleted: this.completedCount,
    };
  }

  private calculateBackgroundLimit(): number {
    const cores = this.deviceInfo.hardwareConcurrency;
    if (this.deviceInfo.isLowEndDevice) {
      return Math.min(2, cores);
    }
    return Math.max(MIN_CONCURRENT_TASKS, Math.min(cores - 1, MAX_BACKGROUND_TASKS));
  }

  private calculateNetworkLimit(): number {
    const cores = this.deviceInfo.hardwareConcurrency;
    if (this.deviceInfo.isLowEndDevice) {
      return Math.min(4, cores * 2);
    }
    return Math.max(MIN_CONCURRENT_TASKS, Math.min(cores * 4, MAX_NETWORK_TASKS));
  }

  private calculateComputeLimit(): number {
    const cores = this.deviceInfo.hardwareConcurrency;
    if (this.deviceInfo.isLowEndDevice) {
      return 1;
    }
    return Math.max(1, Math.min(Math.floor(cores / 2), MAX_COMPUTE_TASKS));
  }

  private log(message: string): void {
    if (this.config.debug) {
      console.log(`[AdaptiveConcurrency] ${message}`);
    }
  }

  private static detectDeviceInfo(): DeviceInfo {
    const hardwareConcurrency = navigator?.hardwareConcurrency ?? 4;
    
    // Device memory API (Chrome only)
    const deviceMemory = (navigator as unknown as { deviceMemory?: number })?.deviceMemory ?? null;
    
    const userAgent = typeof navigator !== 'undefined' ? navigator.userAgent : 'unknown';
    const isWorker = typeof window === 'undefined' && typeof self !== 'undefined';
    
    // Heuristic for low-end devices
    const isLowEndDevice = 
      hardwareConcurrency <= 2 || 
      (deviceMemory !== null && deviceMemory < 4) ||
      /Android [1-6]\./i.test(userAgent);

    return {
      hardwareConcurrency,
      deviceMemory,
      isLowEndDevice,
      userAgent,
      isWorker,
    };
  }
}

// Export a convenience function
export function getAdaptiveConcurrency(config?: AdaptiveConcurrencyConfig): AdaptiveConcurrency {
  return AdaptiveConcurrency.getInstance(config);
}

export default AdaptiveConcurrency;
