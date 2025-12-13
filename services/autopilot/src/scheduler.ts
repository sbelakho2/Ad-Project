import cron from 'node-cron';
import pino from 'pino';
import { config } from './config';
import * as db from './database';

const logger = pino({ level: config.logging.level });

// Track scheduled jobs
const scheduledJobs: cron.ScheduledTask[] = [];

/**
 * Wrapper to run a job with error handling and logging
 */
async function runJob(jobType: string, jobFn: () => Promise<unknown>): Promise<void> {
  const startedAt = new Date();
  
  try {
    logger.info({ jobType }, 'Starting scheduled job');
    const result = await jobFn();
    const completedAt = new Date();
    
    await db.logAutopilotJob(
      jobType,
      'completed',
      typeof result === 'object' ? result as Record<string, unknown> : { result },
      undefined,
      startedAt,
      completedAt
    );
    
    logger.info({ jobType, durationMs: completedAt.getTime() - startedAt.getTime() }, 'Job completed');
  } catch (error) {
    const completedAt = new Date();
    const errorMessage = error instanceof Error ? error.message : String(error);
    
    await db.logAutopilotJob(
      jobType,
      'failed',
      undefined,
      errorMessage,
      startedAt,
      completedAt
    );
    
    logger.error({ jobType, error: errorMessage }, 'Job failed');
  }
}

/**
 * Initialize the scheduler with all cron jobs
 */
export function initializeScheduler(): void {
  logger.info('Initializing scheduler');
  
  // Health check - runs every minute
  const healthCheckSeconds = config.scheduler.healthCheckIntervalSeconds;
  const healthCheckCron = healthCheckSeconds >= 60
    ? `*/${Math.floor(healthCheckSeconds / 60)} * * * *`
    : `*/${healthCheckSeconds} * * * * *`;
  
  const healthJob = cron.schedule(healthCheckCron, async () => {
    await runJob('health_check', db.runHealthCheck);
  });
  scheduledJobs.push(healthJob);
  logger.info({ cron: healthCheckCron }, 'Scheduled health check');
  
  // Partition check - runs every N hours
  const partitionHours = config.scheduler.partitionCheckIntervalHours;
  const partitionJob = cron.schedule(`0 */${partitionHours} * * *`, async () => {
    await runJob('ensure_partitions', db.ensureFuturePartitions);
  });
  scheduledJobs.push(partitionJob);
  logger.info({ interval: `${partitionHours}h` }, 'Scheduled partition check');
  
  // Retention cleanup - runs daily at configured hour
  const retentionHour = config.scheduler.retentionCleanupHour;
  const retentionJob = cron.schedule(`0 ${retentionHour} * * *`, async () => {
    await runJob('retention_cleanup', db.applyRetentionPolicy);
  });
  scheduledJobs.push(retentionJob);
  logger.info({ hour: retentionHour }, 'Scheduled retention cleanup');
  
  // Hourly aggregation - runs every N minutes
  const aggMinutes = config.scheduler.aggregationRunMinutes;
  const hourlyAggJob = cron.schedule(`${aggMinutes} * * * *`, async () => {
    await runJob('hourly_aggregation', async () => {
      return { rows_updated: await db.computeHourlyMetrics() };
    });
  });
  scheduledJobs.push(hourlyAggJob);
  logger.info({ minute: aggMinutes }, 'Scheduled hourly aggregation');
  
  // Daily aggregation - runs at 1 AM
  const dailyAggJob = cron.schedule('0 1 * * *', async () => {
    await runJob('daily_aggregation', async () => {
      return { rows_updated: await db.computeDailyMetrics() };
    });
  });
  scheduledJobs.push(dailyAggJob);
  logger.info('Scheduled daily aggregation at 1 AM');
  
  // Job cleanup - runs weekly on Sunday at 4 AM
  const cleanupJob = cron.schedule('0 4 * * 0', async () => {
    await runJob('job_cleanup', async () => {
      return { deleted: await db.cleanupOldJobs(30) };
    });
  });
  scheduledJobs.push(cleanupJob);
  logger.info('Scheduled weekly job cleanup');
  
  logger.info({ totalJobs: scheduledJobs.length }, 'Scheduler initialized');
}

/**
 * Manually trigger a job by type
 */
export async function triggerJob(jobType: string): Promise<{ success: boolean; error?: string }> {
  try {
    switch (jobType) {
      case 'health_check':
        await runJob(jobType, db.runHealthCheck);
        break;
      case 'ensure_partitions':
        await runJob(jobType, db.ensureFuturePartitions);
        break;
      case 'retention_cleanup':
        await runJob(jobType, db.applyRetentionPolicy);
        break;
      case 'hourly_aggregation':
        await runJob(jobType, async () => ({ rows_updated: await db.computeHourlyMetrics() }));
        break;
      case 'daily_aggregation':
        await runJob(jobType, async () => ({ rows_updated: await db.computeDailyMetrics() }));
        break;
      case 'job_cleanup':
        await runJob(jobType, async () => ({ deleted: await db.cleanupOldJobs(30) }));
        break;
      default:
        return { success: false, error: `Unknown job type: ${jobType}` };
    }
    return { success: true };
  } catch (error) {
    return { success: false, error: error instanceof Error ? error.message : String(error) };
  }
}

/**
 * Get available job types
 */
export function getAvailableJobs(): string[] {
  return [
    'health_check',
    'ensure_partitions',
    'retention_cleanup',
    'hourly_aggregation',
    'daily_aggregation',
    'job_cleanup',
  ];
}

/**
 * Stop all scheduled jobs
 */
export function stopScheduler(): void {
  logger.info('Stopping scheduler');
  scheduledJobs.forEach(job => job.stop());
  scheduledJobs.length = 0;
  logger.info('Scheduler stopped');
}
