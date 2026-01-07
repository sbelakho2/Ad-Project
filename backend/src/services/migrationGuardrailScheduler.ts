/**
 * Migration Guardrail Scheduler
 * 
 * Runs periodic aggregation of shadow outcomes into guardrail snapshots,
 * then evaluates guardrails for all active experiments.
 */

import cron, { ScheduledTask } from 'node-cron';
import { Pool } from 'pg';
import logger from '../utils/logger';
import { MigrationGuardrailSnapshotService } from './migrationGuardrailSnapshotService';
import { MigrationStudioService } from './migrationStudioService';

let snapshotJob: ScheduledTask | null = null;
let evaluationJob: ScheduledTask | null = null;

export interface InitMigrationGuardrailSchedulerParams {
  enabled: boolean;
  /** Cron spec for snapshot aggregation (default: every 15 minutes) */
  snapshotCronSpec?: string;
  /** Cron spec for guardrail evaluation (default: every 30 minutes) */
  evaluationCronSpec?: string;
  pool: Pool;
}

const DEFAULT_SNAPSHOT_CRON = '*/15 * * * *'; // Every 15 minutes
const DEFAULT_EVALUATION_CRON = '5,35 * * * *'; // At 5 and 35 past every hour

export function initMigrationGuardrailScheduler({
  enabled,
  snapshotCronSpec = DEFAULT_SNAPSHOT_CRON,
  evaluationCronSpec = DEFAULT_EVALUATION_CRON,
  pool,
}: InitMigrationGuardrailSchedulerParams): void {
  if (!enabled) {
    logger.info('Migration guardrail scheduler: disabled');
    return;
  }

  // Stop existing jobs if any
  stopMigrationGuardrailScheduler();

  const snapshotService = new MigrationGuardrailSnapshotService(pool);
  const studioService = new MigrationStudioService(pool);

  // === Snapshot Aggregation Job ===
  const runSnapshotAggregation = async (reason: string): Promise<void> => {
    logger.info('Migration guardrail snapshot aggregation: starting', { reason });
    try {
      const stats = await snapshotService.aggregateAllActive();
      logger.info('Migration guardrail snapshot aggregation: complete', { reason, ...stats });

      // Also cleanup old snapshots once per day (on the first run after midnight)
      const hour = new Date().getUTCHours();
      if (hour === 0 || reason === 'startup') {
        const deleted = await snapshotService.cleanupOldSnapshots();
        if (deleted > 0) {
          logger.info('Migration guardrail snapshot cleanup: complete', { deleted });
        }
      }
    } catch (error) {
      logger.error('Migration guardrail snapshot aggregation: failed', {
        reason,
        error: (error as Error).message,
      });
    }
  };

  // Non-blocking startup run
  void runSnapshotAggregation('startup').catch(err => {
    logger.error('Migration guardrail snapshot aggregation: startup failed', {
      error: (err as Error).message,
    });
  });

  snapshotJob = cron.schedule(snapshotCronSpec, async () => {
    await runSnapshotAggregation('scheduled');
  });
  snapshotJob.start();

  // === Guardrail Evaluation Job ===
  const runGuardrailEvaluation = async (reason: string): Promise<void> => {
    logger.info('Migration guardrail evaluation: starting', { reason });
    try {
      // Get all active experiments
      const activeExperiments = await pool.query(
        `SELECT id, publisher_id, name FROM migration_experiments WHERE status = 'active'`
      );

      let evaluated = 0;
      let paused = 0;
      let errors = 0;

      for (const exp of activeExperiments.rows) {
        try {
          const result = await studioService.evaluateGuardrails(
            exp.id,
            exp.publisher_id,
            'automated_scheduler'
          );

          evaluated++;
          if (result.shouldPause) {
            paused++;
            logger.warn('Migration guardrail evaluation: experiment auto-paused', {
              experimentId: exp.id,
              experimentName: exp.name,
              violations: result.violations,
            });
          }
        } catch (error) {
          errors++;
          logger.warn('Migration guardrail evaluation: experiment failed', {
            experimentId: exp.id,
            experimentName: exp.name,
            error: (error as Error).message,
          });
        }
      }

      logger.info('Migration guardrail evaluation: complete', {
        reason,
        total: activeExperiments.rows.length,
        evaluated,
        paused,
        errors,
      });
    } catch (error) {
      logger.error('Migration guardrail evaluation: critical failure', {
        reason,
        error: (error as Error).message,
      });
    }
  };

  // Delay startup evaluation by 2 minutes to allow snapshots to aggregate first
  setTimeout(() => {
    void runGuardrailEvaluation('startup').catch(err => {
      logger.error('Migration guardrail evaluation: startup failed', {
        error: (err as Error).message,
      });
    });
  }, 2 * 60 * 1000);

  evaluationJob = cron.schedule(evaluationCronSpec, async () => {
    await runGuardrailEvaluation('scheduled');
  });
  evaluationJob.start();

  logger.info('Migration guardrail scheduler: started', {
    snapshotCronSpec,
    evaluationCronSpec,
  });
}

export function stopMigrationGuardrailScheduler(): void {
  if (snapshotJob) {
    snapshotJob.stop();
    snapshotJob = null;
  }
  if (evaluationJob) {
    evaluationJob.stop();
    evaluationJob = null;
  }
  logger.info('Migration guardrail scheduler: stopped');
}
