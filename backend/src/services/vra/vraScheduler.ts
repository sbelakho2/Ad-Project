/**
 * VRA Scheduled Jobs
 * 
 * Cron jobs for automated network sync, expected data building, and reconciliation.
 * 
 * CRITICAL FIX: Added expectedJob to automatically populate recon_expected from
 * transparency_receipts. Without this, reconciliation would report 100% underpay
 * forever because the expected data table remained empty.
 */

import cron, { ScheduledTask } from 'node-cron';
import { Pool } from 'pg';
import logger from '../../utils/logger';
import { VraSyncService } from './vraSyncService';
import { buildReconExpected } from './expectedBuilder';

let pgPool: Pool | null = null;
let syncService: VraSyncService | null = null;
let syncJob: ScheduledTask | null = null;
let expectedJob: ScheduledTask | null = null;
let reconJob: ScheduledTask | null = null;
let deepSyncJob: ScheduledTask | null = null;

/**
 * Initialize VRA scheduled jobs
 */
export function initVraScheduler(pool: Pool): void {
  pgPool = pool;
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

  // ========================================================================
  // CRITICAL FIX: Build recon_expected every hour
  // ========================================================================
  // This job was MISSING, causing reconciliation to always show 100% underpay
  // because recon_expected remained empty without manual CLI execution.
  // 
  // Schedule: Every hour at minute 15 (offset from sync to avoid overlap)
  // 15 * * * * = At minute 15 past every hour
  // ========================================================================
  expectedJob = cron.schedule('15 * * * *', async () => {
    logger.info('VRA Scheduler: Starting expected data build job');
    try {
      // Build expected data for the last 6 hours (overlap for resilience)
      const to = new Date();
      const from = new Date(to.getTime() - 6 * 60 * 60 * 1000);
      
      const result = await buildReconExpected(pgPool!, {
        from: from.toISOString(),
        to: to.toISOString(),
        limit: 50000, // Reasonable limit per hourly run
      });
      
      logger.info('VRA Scheduler: Expected data build complete', {
        seen: result.seen,
        written: result.written,
        skipped: result.skipped,
      });
    } catch (err) {
      logger.error('VRA Scheduler: Expected data build failed', { error: (err as Error).message });
    }
  }, {
    timezone: 'UTC',
  });
  expectedJob.stop();

  // Run reconciliation daily at 8am UTC (after networks have posted T-1 data)
  // 0 8 * * * = At 08:00 every day
  reconJob = cron.schedule('0 8 * * *', async () => {
    logger.info('VRA Scheduler: Starting daily reconciliation job');
    try {
      const results = await syncService!.runReconciliationForAll(3); // 3-day lookback
      const withVariance = results.filter(r => Math.abs(r.variancePercent ?? 0) > 2).length;
      logger.info('VRA Scheduler: Reconciliation complete', {
        publishers: results.length,
        withVariance,
        alerts: results.reduce((sum, r) => sum + (r.alerts?.length ?? 0), 0),
      });
    } catch (err) {
      logger.error('VRA Scheduler: Reconciliation job failed', { error: (err as Error).message });
    }
  }, {
    timezone: 'UTC',
  });
  // Stop it immediately
  reconJob.stop();

  // ========================================================================
  // DEEP SYNC: Weekly 30-day lookback to catch retroactive adjustments
  // ========================================================================
  // Networks may adjust revenue retroactively (e.g., IVT clawbacks) up to 
  // 30+ days after the event. The regular 3-day lookback misses these.
  // 
  // Schedule: Every Sunday at 2am UTC
  // 0 2 * * 0 = At 02:00 on Sunday
  // ========================================================================
  deepSyncJob = cron.schedule('0 2 * * 0', async () => {
    logger.info('VRA Scheduler: Starting weekly deep sync job (30-day lookback)');
    try {
      // Get all connections and force a 30-day lookback
      const connections = await pgPool!.query(
        `SELECT DISTINCT publisher_id FROM vra_network_connections WHERE status = 'active'`
      );
      
      let synced = 0;
      let failed = 0;
      
      for (const row of connections.rows) {
        try {
          // Temporarily override the lookback days for this sync
          await pgPool!.query(
            `UPDATE vra_network_connections 
             SET sync_lookback_days = 30
             WHERE publisher_id = $1 AND status = 'active'`,
            [row.publisher_id]
          );
          
          // Sync all due connections for this publisher
          const results = await syncService!.syncAllDue();
          synced += results.filter(r => r.status === 'success').length;
          failed += results.filter(r => r.status === 'error').length;
          
          // Restore normal lookback
          await pgPool!.query(
            `UPDATE vra_network_connections 
             SET sync_lookback_days = COALESCE(
               (SELECT default_lookback_days FROM vra_network_configs WHERE network_type = vra_network_connections.network_type),
               3
             )
             WHERE publisher_id = $1 AND status = 'active'`,
            [row.publisher_id]
          );
        } catch (err) {
          logger.warn('VRA Scheduler: Deep sync failed for publisher', {
            publisherId: row.publisher_id,
            error: (err as Error).message,
          });
          failed++;
        }
      }
      
      logger.info('VRA Scheduler: Deep sync complete', { synced, failed });
    } catch (err) {
      logger.error('VRA Scheduler: Deep sync job failed', { error: (err as Error).message });
    }
  }, {
    timezone: 'UTC',
  });
  deepSyncJob.stop();

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
  if (expectedJob) {
    expectedJob.start();
    logger.info('VRA Scheduler: Expected data build job started (every hour)');
  }
  if (reconJob) {
    reconJob.start();
    logger.info('VRA Scheduler: Reconciliation job started (daily at 8am UTC)');
  }
  if (deepSyncJob) {
    deepSyncJob.start();
    logger.info('VRA Scheduler: Deep sync job started (weekly on Sunday at 2am UTC)');
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
  if (expectedJob) {
    expectedJob.stop();
    logger.info('VRA Scheduler: Expected data build job stopped');
  }
  if (reconJob) {
    reconJob.stop();
    logger.info('VRA Scheduler: Reconciliation job stopped');
  }
  if (deepSyncJob) {
    deepSyncJob.stop();
    logger.info('VRA Scheduler: Deep sync job stopped');
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
 * Manually trigger expected data build
 */
export async function triggerManualExpectedBuild(
  from?: Date,
  to?: Date
): Promise<{ seen: number; written: number; skipped: number }> {
  if (!pgPool) {
    throw new Error('VRA Scheduler not initialized');
  }
  
  const endDate = to || new Date();
  const startDate = from || new Date(endDate.getTime() - 24 * 60 * 60 * 1000); // default 24 hours
  
  return buildReconExpected(pgPool, {
    from: startDate.toISOString(),
    to: endDate.toISOString(),
    limit: 100000,
  });
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

/**
 * Manually trigger deep sync (30-day lookback) for a publisher
 */
export async function triggerDeepSync(publisherId: string): Promise<void> {
  if (!syncService || !pgPool) {
    throw new Error('VRA Scheduler not initialized');
  }
  
  // Temporarily set 30-day lookback
  await pgPool.query(
    `UPDATE vra_network_connections 
     SET sync_lookback_days = 30
     WHERE publisher_id = $1 AND status = 'active'`,
    [publisherId]
  );
  
  try {
    await syncService.syncAllDue();
  } finally {
    // Restore normal lookback
    await pgPool.query(
      `UPDATE vra_network_connections 
       SET sync_lookback_days = COALESCE(
         (SELECT default_lookback_days FROM vra_network_configs WHERE network_type = vra_network_connections.network_type),
         3
       )
       WHERE publisher_id = $1 AND status = 'active'`,
      [publisherId]
    );
  }
}

export default {
  initVraScheduler,
  startVraScheduler,
  stopVraScheduler,
  triggerManualSync,
  triggerManualExpectedBuild,
  triggerManualReconciliation,
  triggerDeepSync,
};
