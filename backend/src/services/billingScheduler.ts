import cron, { ScheduledTask } from 'node-cron';
import { logger } from '../utils/logger';
import { usageMeteringService } from './billing/UsageMeteringService';
import { reconciliationService } from './reconciliationService';
import { dunningManagementService } from '../../services/billing/DunningManagementService';

export interface InitBillingSchedulerParams {
  enabled: boolean;
  usageLimitCronSpec: string;
  usageSyncCronSpec: string;
  dunningCronSpec: string;
  reconciliationCronSpec: string;
  reconciliationEnabled: boolean;
}

let usageLimitJob: ScheduledTask | null = null;
let usageSyncJob: ScheduledTask | null = null;
let dunningJob: ScheduledTask | null = null;
let reconciliationJob: ScheduledTask | null = null;

export function initBillingScheduler(params: InitBillingSchedulerParams): void {
  const {
    enabled,
    usageLimitCronSpec,
    usageSyncCronSpec,
    dunningCronSpec,
    reconciliationCronSpec,
    reconciliationEnabled,
  } = params;

  if (!enabled) {
    logger.info('Billing scheduler disabled');
    return;
  }

  stopBillingScheduler();

  usageLimitJob = cron.schedule(usageLimitCronSpec, async () => {
    try {
      await usageMeteringService.checkUsageLimits();
      logger.info('Billing scheduler: usage limits checked');
    } catch (error) {
      logger.error('Billing scheduler: usage limit check failed', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  });

  usageSyncJob = cron.schedule(usageSyncCronSpec, async () => {
    try {
      await usageMeteringService.syncUsageToStripe();
      logger.info('Billing scheduler: usage synced to Stripe');
    } catch (error) {
      logger.error('Billing scheduler: usage sync failed', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  });

  dunningJob = cron.schedule(dunningCronSpec, async () => {
    try {
      await dunningManagementService.processDunningRetries();
      logger.info('Billing scheduler: dunning retries processed');
    } catch (error) {
      logger.error('Billing scheduler: dunning retries failed', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  });

  if (reconciliationEnabled) {
    reconciliationJob = cron.schedule(reconciliationCronSpec, async () => {
      try {
        await reconciliationService.reconcile();
        logger.info('Billing scheduler: reconciliation complete');
      } catch (error) {
        logger.error('Billing scheduler: reconciliation failed', {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    });
  }

  logger.info('Billing scheduler started', {
    usageLimitCronSpec,
    usageSyncCronSpec,
    dunningCronSpec,
    reconciliationCronSpec: reconciliationEnabled ? reconciliationCronSpec : 'disabled',
  });
}

export function stopBillingScheduler(): void {
  usageLimitJob?.stop();
  usageSyncJob?.stop();
  dunningJob?.stop();
  reconciliationJob?.stop();

  usageLimitJob = null;
  usageSyncJob = null;
  dunningJob = null;
  reconciliationJob = null;
}
