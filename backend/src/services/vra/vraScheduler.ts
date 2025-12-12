/**
 * VRA Scheduled Jobs
 * 
 * Cron jobs for automated network sync and reconciliation
 */

import cron, { ScheduledTask } from 'node-cron';
import { Pool } from 'pg';
import logger from '../../utils/logger';
import { VraSyncService } from './vraSyncService';

let syncService: VraSyncService | null = null;
let syncJob: ScheduledTask | null = null;
let reconJob: ScheduledTask | null = null;

/**
 * Initialize VRA scheduled jobs
 */
export function initVraScheduler(pool: Pool): void {
  syncService = new VraSyncService(pool);

  // Sync network data every 6 hours
  // 0 */6 * * * = At minute 0 past every 6th hour
  syncJob = cron.schedule('0 */6 * * *', async () => {
    logger.info('VRA Scheduler: Starting network sync job');
    try {
      const results = await syncService!.syncAllDue();
      const successful = results.filter(r => r.status === 'success').length;
      const failed = results.filter(r => r.status === 'error').length;
      logger.info('VRA Scheduler: Network sync complete', { successful, failed, total: results.length });
    } catch (err) {
      logger.error('VRA Scheduler: Network sync job failed', { error: (err as Error).message });
    }
  }, {
    timezone: 'UTC',
  });
  // Stop it immediately - we'll start manually when enabled
  syncJob.stop();

  // Run reconciliation daily at 8am UTC (after networks have posted T-1 data)
  // 0 8 * * * = At 08:00 every day
  reconJob = cron.schedule('0 8 * * *', async () => {
    logger.info('VRA Scheduler: Starting daily reconciliation job');
    try {
      const results = await syncService!.runReconciliationForAll(3); // 3-day lookback
      const withVariance = results.filter(r => Math.abs(r.variancePercent) > 2).length;
      logger.info('VRA Scheduler: Reconciliation complete', {
        publishers: results.length,
        withVariance,
        alerts: results.reduce((sum, r) => sum + r.alerts.length, 0),
      });
    } catch (err) {
      logger.error('VRA Scheduler: Reconciliation job failed', { error: (err as Error).message });
    }
  }, {
    timezone: 'UTC',
  });
  // Stop it immediately
  reconJob.stop();

  // Check if VRA is enabled via environment
  if (process.env.VRA_SCHEDULER_ENABLED === 'true') {
    startVraScheduler();
  }

  logger.info('VRA Scheduler initialized', { 
    enabled: process.env.VRA_SCHEDULER_ENABLED === 'true' 
  });
}

/**
 * Start the VRA scheduler
 */
export function startVraScheduler(): void {
  if (syncJob) {
    syncJob.start();
    logger.info('VRA Scheduler: Sync job started (every 6 hours)');
  }
  if (reconJob) {
    reconJob.start();
    logger.info('VRA Scheduler: Reconciliation job started (daily at 8am UTC)');
  }
}

/**
 * Stop the VRA scheduler
 */
export function stopVraScheduler(): void {
  if (syncJob) {
    syncJob.stop();
    logger.info('VRA Scheduler: Sync job stopped');
  }
  if (reconJob) {
    reconJob.stop();
    logger.info('VRA Scheduler: Reconciliation job stopped');
  }
}

/**
 * Manually trigger a sync for all due connections
 */
export async function triggerManualSync(): Promise<void> {
  if (!syncService) {
    throw new Error('VRA Scheduler not initialized');
  }
  await syncService.syncAllDue();
}

/**
 * Manually trigger reconciliation for a publisher
 */
export async function triggerManualReconciliation(
  publisherId: string,
  windowStart?: Date,
  windowEnd?: Date
): Promise<void> {
  if (!syncService) {
    throw new Error('VRA Scheduler not initialized');
  }
  
  const end = windowEnd || new Date();
  end.setHours(0, 0, 0, 0);
  const start = windowStart || new Date(end);
  start.setDate(start.getDate() - 3);
  
  await syncService.runReconciliation(publisherId, start, end);
}

export default {
  initVraScheduler,
  startVraScheduler,
  stopVraScheduler,
  triggerManualSync,
  triggerManualReconciliation,
};
