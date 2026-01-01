import cron, { ScheduledTask } from 'node-cron';
import logger from '../utils/logger';
import { runAnalyticsRollups } from './analyticsRollupService';

let rollupJob: ScheduledTask | null = null;

const utcMidnight = (d: Date): Date => new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));

export interface InitAnalyticsRollupSchedulerParams {
  enabled: boolean;
  cronSpec: string;
  lookbackDays: number;
}

export function initAnalyticsRollupScheduler({
  enabled,
  cronSpec,
  lookbackDays,
}: InitAnalyticsRollupSchedulerParams): void {
  if (!enabled) {
    logger.info('Analytics rollups: scheduler disabled');
    return;
  }

  if (rollupJob) {
    rollupJob.stop();
    rollupJob = null;
  }

  const safeLookbackDays = Number.isFinite(lookbackDays) && lookbackDays > 0 ? Math.floor(lookbackDays) : 35;

  const runOnce = async (reason: string): Promise<void> => {
    const now = new Date();
    const endExclusive = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1)));
    const startInclusive = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - safeLookbackDays)));

    logger.info('Analytics rollups: run starting', {
      reason,
      lookbackDays: safeLookbackDays,
      startInclusive: startInclusive.toISOString(),
      endExclusive: endExclusive.toISOString(),
    });

    await runAnalyticsRollups({ startInclusive, endExclusive });
  };

  // Non-blocking warm run at startup.
  void runOnce('startup').catch(err => {
    logger.error('Analytics rollups: startup run failed', { error: (err as Error).message });
  });

  rollupJob = cron.schedule(cronSpec, async () => {
    try {
      await runOnce('scheduled');
    } catch (err) {
      logger.error('Analytics rollups: scheduled run failed', { error: (err as Error).message });
    }
  });

  rollupJob.start();
  logger.info('Analytics rollups: scheduler started', { cronSpec, lookbackDays: safeLookbackDays });
}

export function stopAnalyticsRollupScheduler(): void {
  if (!rollupJob) return;
  rollupJob.stop();
  rollupJob = null;
  logger.info('Analytics rollups: scheduler stopped');
}
