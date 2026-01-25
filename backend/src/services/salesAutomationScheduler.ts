import cron, { ScheduledTask } from 'node-cron';
import { Pool } from 'pg';
import logger from '../utils/logger';
import { AutomationEngine } from '../../services/sales-automation/AutomationEngine';

export interface InitSalesAutomationSchedulerParams {
  enabled: boolean;
  cronSpec: string;
  runOnStartup: boolean;
  databaseUrl: string;
  defaultMailbox: string;
  maxDailyEmails: number;
  enableSelfLearning: boolean;
  enableAutonomousMode: boolean;
}

let automationJob: ScheduledTask | null = null;
let engine: AutomationEngine | null = null;

const REQUIRED_TABLES = [
  'sales_contacts',
  'sales_prospects',
  'outreach_sequences',
  'outreach_sequence_steps',
  'outreach_enrollments',
  'outreach_emails',
];

async function validateSalesAutomationSchema(databaseUrl: string): Promise<{ ok: boolean; missing: string[] }> {
  const pool = new Pool({ connectionString: databaseUrl });
  try {
    const missing: string[] = [];
    for (const table of REQUIRED_TABLES) {
      const result = await pool.query('SELECT to_regclass($1) AS name', [`public.${table}`]);
      if (!result.rows[0]?.name) {
        missing.push(table);
      }
    }
    return { ok: missing.length === 0, missing };
  } finally {
    await pool.end();
  }
}

export function initSalesAutomationScheduler({
  enabled,
  cronSpec,
  runOnStartup,
  databaseUrl,
  defaultMailbox,
  maxDailyEmails,
  enableSelfLearning,
  enableAutonomousMode,
}: InitSalesAutomationSchedulerParams): void {
  if (!enabled) {
    logger.info('Sales automation: scheduler disabled');
    return;
  }

  if (!enableAutonomousMode) {
    logger.info('Sales automation: autonomous mode disabled');
    return;
  }

  if (!databaseUrl) {
    logger.error('Sales automation: DATABASE_URL not configured');
    return;
  }

  if (automationJob) {
    automationJob.stop();
    automationJob = null;
  }

  void (async () => {
    const schemaStatus = await validateSalesAutomationSchema(databaseUrl);
    if (!schemaStatus.ok) {
      logger.warn('Sales automation: missing database tables, scheduler not started', {
        missing: schemaStatus.missing,
      });
      return;
    }

    engine = new AutomationEngine({
      databaseUrl,
      defaultMailbox,
      maxDailyEmails,
      enableSelfLearning,
      enableAutonomousMode,
    });

    await engine.initialize();

    if (runOnStartup) {
      try {
        await engine.runCycle();
      } catch (error) {
        logger.error('Sales automation: startup run failed', {
          error: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    }

    automationJob = cron.schedule(cronSpec, async () => {
      if (!engine) return;
      try {
        await engine.runCycle();
      } catch (error) {
        logger.error('Sales automation: scheduled run failed', {
          error: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    });

    automationJob.start();
    logger.info('Sales automation: scheduler started', {
      cronSpec,
      runOnStartup,
      maxDailyEmails,
      defaultMailbox,
      enableSelfLearning,
      enableAutonomousMode,
    });
  })().catch((error) => {
    logger.error('Sales automation: scheduler initialization failed', {
      error: error instanceof Error ? error.message : 'Unknown error',
    });
  });
}

export function stopSalesAutomationScheduler(): void {
  if (automationJob) {
    automationJob.stop();
    automationJob = null;
  }
  engine = null;
  logger.info('Sales automation: scheduler stopped');
}
