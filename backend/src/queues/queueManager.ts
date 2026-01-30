import { Queue, Worker, JobsOptions, RedisOptions } from 'bullmq';
import { redis } from '../utils/redis';
import logger from '../utils/logger';

export enum QueueName {
  ANALYTICS_INGEST = 'analytics_ingest',
  ANALYTICS_AGGREGATION = 'analytics_aggregation',
  DATA_EXPORT = 'data_export',
  PRIVACY = 'privacy',
  REPORT_GENERATION = 'report_generation',
  METRICS_CALCULATION = 'metrics_calculation',
  CACHE_WARMING = 'cache_warming',
  CLEANUP_TASKS = 'cleanup_tasks',
  // Dead Letter Queue for failed jobs requiring manual investigation
  DEAD_LETTER = 'dead_letter',
}

/**
 * Dead Letter Queue (DLQ) entry structure
 * Failed jobs are moved here after exhausting retries for forensic analysis
 */
export type DeadLetterJob = {
  originalQueue: QueueName;
  originalJobId: string;
  originalJobName: string;
  payload: Record<string, unknown>;
  failureReason: string;
  failureStack?: string;
  attemptsMade: number;
  firstFailedAt: string;
  lastFailedAt: string;
};

export type AnalyticsAggregationJob = {
  publisherId: string;
  startDate: string;
  endDate: string;
  granularity: 'hour' | 'day' | 'week';
};

export type DataExportJob = {
  jobId: string;
  publisherId: string;
  format: 'csv' | 'json';
  startDate: string;
  endDate: string;
  filters?: Record<string, unknown>;
};

export type PrivacyJob = {
  kind: 'export' | 'delete';
  tenantId: string;
  userId: string;
  requestId?: string;
  format?: 'json' | 'csv';
};

type QueueWorkerOptions = {
  concurrency?: number;
  limiter?: {
    max: number;
    duration: number;
  };
};

class QueueManagerImpl {
  private initialized = false;
  private queues = new Map<QueueName, Queue>();
  private workers = new Map<QueueName, Worker>();
  private redisOptions: RedisOptions | null = null;

  private buildRedisOptions(redisUrl: string): RedisOptions {
    try {
      const parsed = new URL(redisUrl);
      const options: RedisOptions = {
        host: parsed.hostname,
        port: parsed.port ? Number(parsed.port) : 6379,
      };
      if (parsed.username) {
        options.username = decodeURIComponent(parsed.username);
      }
      if (parsed.password) {
        options.password = decodeURIComponent(parsed.password);
      }
      if (parsed.pathname && parsed.pathname.length > 1) {
        const db = Number(parsed.pathname.slice(1));
        if (!Number.isNaN(db)) {
          options.db = db;
        }
      }
      return options;
    } catch (error) {
      logger.warn('[QueueManager] Failed to parse REDIS_URL for BullMQ, falling back to defaults', { error });
      return { host: '127.0.0.1', port: 6379 };
    }
  }

  private ensureQueue(queueName: QueueName): Queue {
    if (!this.initialized || !this.redisOptions) {
      throw new Error('Queue manager not initialized');
    }

    let queue = this.queues.get(queueName);
    if (!queue) {
      queue = new Queue(queueName, { connection: this.redisOptions });
      this.queues.set(queueName, queue);
      logger.info('[QueueManager] BullMQ queue ready', { queueName });
    }

    return queue;
  }

  /**
   * Ready once BullMQ connection metadata has been configured.
   */
  isReady(): boolean {
    return this.initialized;
  }

  getQueue(name: QueueName): Queue | null {
    if (!this.initialized) {
      return null;
    }

    try {
      return this.ensureQueue(name);
    } catch (error) {
      logger.error('[QueueManager] Failed to access queue', { queueName: name, error });
      return null;
    }
  }

  async initialize(): Promise<void> {
    if (this.initialized) return;

    const redisUrl = process.env.REDIS_URL || 'redis://localhost:6379';
    this.redisOptions = this.buildRedisOptions(redisUrl);
    this.initialized = true;
    logger.info('[QueueManager] BullMQ initialized', { redisUrl });
  }

  registerWorker(
    queueName: QueueName,
    handler: (job: any) => Promise<unknown>,
    options?: QueueWorkerOptions
  ): void {
    if (!this.initialized || !this.redisOptions) {
      logger.warn('[QueueManager] registerWorker called before initialization', { queueName });
      return;
    }

    const worker = new Worker(queueName, handler as any, {
      connection: this.redisOptions,
      concurrency: options?.concurrency ?? 1,
      limiter: options?.limiter,
    });

    worker.on('error', (error) => {
      logger.error('[QueueManager] Worker error', { queueName, error });
    });

    worker.on('failed', (job, error) => {
      const maxAttempts = job?.opts.attempts ?? 3;
      const attemptsMade = job?.attemptsMade ?? 0;
      const willRetry = attemptsMade < maxAttempts;
      
      if (willRetry) {
        logger.warn('[QueueManager] Job failed, will retry', { 
          queueName, 
          jobId: job?.id, 
          attempt: attemptsMade,
          maxAttempts,
          nextRetryIn: job?.opts.backoff ? `exponential delay` : 'immediate',
          error: error.message 
        });
      } else {
        logger.error('[QueueManager] Job permanently failed, moving to DLQ', { 
          queueName, 
          jobId: job?.id, 
          attemptsMade,
          error: error.message 
        });
      }
      
      // Move permanently failed jobs to Dead Letter Queue
      if (job && !willRetry) {
        this.moveToDeadLetterQueue(queueName, job, error).catch((dlqError) => {
          logger.error('[QueueManager] Failed to move job to DLQ', { queueName, jobId: job.id, dlqError });
        });
      }
    });

    worker.on('completed', (job) => {
      logger.debug('[QueueManager] Job completed', { queueName, jobId: job.id });
    });

    this.workers.set(queueName, worker);
    logger.info('[QueueManager] Worker registered', {
      queueName,
      handler: handler?.name || 'anonymous',
      concurrency: options?.concurrency ?? 1,
    });
  }

  scheduleRecurringJob(
    queueName: QueueName,
    jobName: string,
    data: Record<string, unknown>,
    cronExpression: string
  ): void {
    if (!this.initialized) {
      logger.warn('[QueueManager] scheduleRecurringJob called before initialization', { queueName });
      return;
    }

    const queue = this.ensureQueue(queueName);
    const repeatJobId = `${jobName}:${cronExpression}`;

    void queue.add(jobName, data, {
      jobId: repeatJobId,
      repeat: { pattern: cronExpression },
      removeOnComplete: true,
      removeOnFail: { age: 7 * 24 * 3600 },
    }).then(() => {
      logger.info('[QueueManager] Scheduled recurring job', { queueName, jobName, cron: cronExpression });
    }).catch((error) => {
      logger.error('[QueueManager] Failed to schedule recurring job', { queueName, jobName, error });
    });
  }

  async shutdown(): Promise<void> {
    const workerClose = Array.from(this.workers.values()).map(async (worker) => {
      try { await worker.close(); } catch (error) { logger.warn('[QueueManager] Failed to close worker', { error }); }
    });
    const queueClose = Array.from(this.queues.values()).map(async (queue) => {
      try { await queue.close(); } catch (error) { logger.warn('[QueueManager] Failed to close queue', { error }); }
    });

    await Promise.all([...workerClose, ...queueClose]);
    this.workers.clear();
    this.queues.clear();
    this.redisOptions = null;
    this.initialized = false;
    logger.info('[QueueManager] Shutdown complete');
  }

  /**
   * Get comprehensive queue metrics
   */
  async getQueueMetrics(queueName: QueueName): Promise<any> {
    if (!this.initialized) {
      return null;
    }

    const queue = this.ensureQueue(queueName);
    const counts = await queue.getJobCounts('waiting', 'active', 'delayed', 'failed', 'completed');
    const paused = await queue.isPaused();

    return {
      name: queueName,
      waiting: counts.waiting ?? 0,
      active: counts.active ?? 0,
      delayed: counts.delayed ?? 0,
      failed: counts.failed ?? 0,
      completed: counts.completed ?? 0,
      paused,
    };
  }

  /**
   * Add a job to the queue with full metadata tracking
   */
  async addJob(
    queueName: QueueName, 
    jobName: string, 
    data: Record<string, unknown>,
    options?: Record<string, unknown>
  ): Promise<any> {
    if (!this.initialized) {
      throw new Error('Queue manager not ready');
    }

    const queue = this.ensureQueue(queueName);
    try {
      const job = await queue.add(jobName, data, this.normalizeJobOptions(options));
      return job;
    } catch (error) {
      logger.error('Failed to add job', { queueName, jobName, error });
      throw error;
    }
  }

  /**
   * Get job details by ID
   */
  async getJob(queueName: QueueName, jobId: string): Promise<any> {
    if (!this.initialized) {
      return null;
    }

    const queue = this.ensureQueue(queueName);
    try {
      return await queue.getJob(jobId);
    } catch (error) {
      logger.error('Failed to get job', { queueName, jobId, error });
      return null;
    }
  }

  /**
   * Remove a job from the queue
   */
  async removeJob(queueName: QueueName, jobId: string): Promise<boolean> {
    if (!this.initialized) {
      return false;
    }

    const queue = this.ensureQueue(queueName);
    try {
      const job = await queue.getJob(jobId);
      if (!job) return false;
      await job.remove();
      logger.debug('Job removed from queue', { queueName, jobId });
      return true;
    } catch (error) {
      logger.error('Failed to remove job', { queueName, jobId, error });
      return false;
    }
  }

  /**
   * Pause queue - prevents new jobs from being processed
   */
  async pauseQueue(queueName: QueueName): Promise<void> {
    if (!this.initialized) {
      throw new Error('Queue manager not ready');
    }

    const queue = this.ensureQueue(queueName);
    await queue.pause();
    logger.info('Queue paused', { queueName });
  }

  /**
   * Resume queue - allows jobs to be processed again
   */
  async resumeQueue(queueName: QueueName): Promise<void> {
    if (!this.initialized) {
      throw new Error('Queue manager not ready');
    }

    const queue = this.ensureQueue(queueName);
    await queue.resume();
    logger.info('Queue resumed', { queueName });
  }

  /**
   * Check if queue is paused
   */
  async isQueuePaused(queueName: QueueName): Promise<boolean> {
    if (!this.initialized) {
      return false;
    }

    const queue = this.ensureQueue(queueName);
    return queue.isPaused();
  }

  /**
   * Clean old jobs from the queue based on grace period
   */
  async cleanQueue(
    queueName: QueueName, 
    gracePeriodSeconds: number = 3600,
    limit: number = 1000,
    type: string = 'completed'
  ): Promise<number> {
    if (!this.initialized) {
      return 0;
    }

    const queue = this.ensureQueue(queueName);
    const status = this.normalizeCleanStatus(type);

    try {
      const cleaned = await queue.clean(gracePeriodSeconds * 1000, limit, status);
      logger.info('Queue cleaned', { queueName, type: status, cleanedCount: cleaned.length, gracePeriodSeconds });
      return cleaned.length;
    } catch (error) {
      logger.error('Failed to clean queue', { queueName, type: status, error });
      return 0;
    }
  }

  private normalizeJobOptions(options?: Record<string, unknown>): JobsOptions {
    const defaults: JobsOptions = {
      // Default retry configuration - prevents immediate DLQ escalation
      attempts: 3,
      backoff: {
        type: 'exponential',
        delay: 2000, // Start with 2s, then 4s, then 8s
      },
      removeOnComplete: { age: 24 * 3600, count: 1000 },
      removeOnFail: { age: 7 * 24 * 3600 },
    };

    if (!options) {
      return defaults;
    }

    return {
      ...defaults,
      ...(options as JobsOptions),
    };
  }

  private normalizeCleanStatus(type: string): 'completed' | 'wait' | 'active' | 'delayed' | 'failed' {
    switch (type) {
      case 'wait':
      case 'waiting':
        return 'wait';
      case 'active':
        return 'active';
      case 'delayed':
        return 'delayed';
      case 'failed':
        return 'failed';
      default:
        return 'completed';
    }
  }

  /**
   * Move a permanently failed job to the Dead Letter Queue for forensic analysis
   */
  private async moveToDeadLetterQueue(originalQueue: QueueName, job: any, error: Error): Promise<void> {
    if (originalQueue === QueueName.DEAD_LETTER) {
      // Don't move DLQ jobs to DLQ again
      return;
    }

    const dlqPayload: DeadLetterJob = {
      originalQueue,
      originalJobId: job.id,
      originalJobName: job.name,
      payload: job.data,
      failureReason: error.message,
      failureStack: error.stack,
      attemptsMade: job.attemptsMade,
      firstFailedAt: new Date(job.processedOn ?? Date.now()).toISOString(),
      lastFailedAt: new Date().toISOString(),
    };

    try {
      const dlq = this.ensureQueue(QueueName.DEAD_LETTER);
      await dlq.add(`dlq:${originalQueue}:${job.name}`, dlqPayload, {
        removeOnComplete: false, // Keep DLQ jobs for investigation
        removeOnFail: false,
      });
      logger.warn('[QueueManager] Job moved to Dead Letter Queue', {
        originalQueue,
        jobId: job.id,
        jobName: job.name,
        failureReason: error.message,
      });
    } catch (dlqError) {
      logger.error('[QueueManager] Failed to add job to DLQ', { originalQueue, jobId: job.id, dlqError });
    }
  }

  /**
   * Get all jobs in the Dead Letter Queue for investigation
   */
  async getDeadLetterJobs(limit: number = 100): Promise<any[]> {
    if (!this.initialized) {
      return [];
    }

    try {
      const dlq = this.ensureQueue(QueueName.DEAD_LETTER);
      const jobs = await dlq.getJobs(['waiting', 'failed'], 0, limit);
      return jobs.map((job) => ({
        id: job.id,
        name: job.name,
        data: job.data,
        timestamp: job.timestamp,
      }));
    } catch (error) {
      logger.error('[QueueManager] Failed to get DLQ jobs', { error });
      return [];
    }
  }

  /**
   * Retry a job from the Dead Letter Queue
   */
  async retryDeadLetterJob(dlqJobId: string): Promise<boolean> {
    if (!this.initialized) {
      return false;
    }

    try {
      const dlq = this.ensureQueue(QueueName.DEAD_LETTER);
      const dlqJob = await dlq.getJob(dlqJobId);
      if (!dlqJob) {
        logger.warn('[QueueManager] DLQ job not found', { dlqJobId });
        return false;
      }

      const dlqData = dlqJob.data as DeadLetterJob;
      const originalQueue = this.ensureQueue(dlqData.originalQueue);
      
      // Re-add to original queue with fresh attempts
      await originalQueue.add(dlqData.originalJobName, dlqData.payload, {
        attempts: 3,
        backoff: { type: 'exponential', delay: 1000 },
      });

      // Remove from DLQ after successful retry
      await dlqJob.remove();
      logger.info('[QueueManager] DLQ job retried', { dlqJobId, originalQueue: dlqData.originalQueue });
      return true;
    } catch (error) {
      logger.error('[QueueManager] Failed to retry DLQ job', { dlqJobId, error });
      return false;
    }
  }
}

export const queueManager = new QueueManagerImpl();
export type QueueManager = typeof queueManager;
