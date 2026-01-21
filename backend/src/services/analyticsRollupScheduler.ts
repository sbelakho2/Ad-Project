import cron, { ScheduledTask } from 'node-cron';
import logger from '../utils/logger';
import { runAnalyticsRollups } from './analyticsRollupService';

let realtimeJob: ScheduledTask | null = null;
let finalizeJob: ScheduledTask | null = null;

const utcMidnight = (d: Date): Date => new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));

export interface InitAnalyticsRollupSchedulerParams {
  enabled: boolean;
  finalizeCronSpec: string;
  realtimeCronSpec: string;
  lookbackDays: number;
  realtimeLookbackHours: number;
  startupBackfillEnabled: boolean;
}

export function initAnalyticsRollupScheduler({
  enabled,
  finalizeCronSpec,
  realtimeCronSpec,
  lookbackDays,
  realtimeLookbackHours,
  startupBackfillEnabled,
}: InitAnalyticsRollupSchedulerParams): void {
  if (!enabled) {
    logger.info('Analytics rollups: scheduler disabled');
    return;
  }

  if (realtimeJob) {
    realtimeJob.stop();
    realtimeJob = null;
  }

  if (finalizeJob) {
    finalizeJob.stop();
    finalizeJob = null;
  }

  const safeLookbackDays = Number.isFinite(lookbackDays) && lookbackDays > 0 ? Math.floor(lookbackDays) : 35;
  const safeRealtimeHours = Number.isFinite(realtimeLookbackHours) && realtimeLookbackHours > 0 ? Math.floor(realtimeLookbackHours) : 6;

  const runRange = async (reason: string, startInclusive: Date, endExclusive: Date): Promise<void> => {
    const now = new Date();
    const startIso = startInclusive.toISOString();
    const endIso = endExclusive.toISOString();

    logger.info('Analytics rollups: run starting', {
      reason,
      now: now.toISOString(),
      lookbackDays: safeLookbackDays,
      startInclusive: startIso,
      endExclusive: endIso,
    });

    await runAnalyticsRollups({ startInclusive, endExclusive });
  };

  const utcToday = (): { start: Date; end: Date } => {
    const now = new Date();
    const start = utcMidnight(now);
    const end = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1)));
    return { start, end };
  };

  const utcYesterday = (): { start: Date; end: Date } => {
    const now = new Date();
    const end = utcMidnight(now);
    const start = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - 1)));
    return { start, end };
  };

  // Non-blocking warm run at startup: backfill the most recent lookback window.
  // This is the only time we intentionally touch multiple days at once.
  if (startupBackfillEnabled) {
    void (async () => {
      const now = new Date();
      const end = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1)));
      const start = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - safeLookbackDays)));
      await runRange('startup-backfill', start, end);
    })().catch(err => {
      logger.error('Analytics rollups: startup run failed', { error: (err as Error).message });
    });
  }

  realtimeJob = cron.schedule(realtimeCronSpec, async () => {
    try {
      const { end } = utcToday();
      const now = new Date();
      const windowStart = new Date(now.getTime() - safeRealtimeHours * 60 * 60 * 1000);
      const start = windowStart < end ? windowStart : end;
      await runRange('scheduled-realtime-today', start, end);
    } catch (err) {
      logger.error('Analytics rollups: realtime scheduled run failed', { error: (err as Error).message });
    }
  });

  finalizeJob = cron.schedule(finalizeCronSpec, async () => {
    try {
      const { start, end } = utcYesterday();
      await runRange('scheduled-finalize-yesterday', start, end);
    } catch (err) {
      logger.error('Analytics rollups: finalize scheduled run failed', { error: (err as Error).message });
    }
  });

  realtimeJob.start();
  finalizeJob.start();
  logger.info('Analytics rollups: scheduler started', {
    realtimeCronSpec,
    finalizeCronSpec,
    lookbackDays: safeLookbackDays,
    realtimeLookbackHours: safeRealtimeHours,
    startupBackfillEnabled,
  });
}

export function stopAnalyticsRollupScheduler(): void {
  if (realtimeJob) {
    realtimeJob.stop();
    realtimeJob = null;
  }

  if (finalizeJob) {
    finalizeJob.stop();
    finalizeJob = null;
  }
  logger.info('Analytics rollups: scheduler stopped');
}
