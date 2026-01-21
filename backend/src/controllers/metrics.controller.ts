import { Request, Response, NextFunction } from 'express';
import { AppError } from '../middleware/errorHandler';
import { usageMeteringService } from '../services/billing/UsageMeteringService';
import { BillingAuditTrailService } from '../services/billingAuditTrailService';
import pool from '../utils/postgres';
import { query } from '../utils/postgres';
import { logger } from '../utils/logger';

const billingAuditTrailService = new BillingAuditTrailService(pool);

type SdkContext = {
  publisherId?: string;
  appId?: string;
  environment?: string;
  keyId?: string;
};

const coerceInt = (value: unknown): number => {
  if (typeof value !== 'number' || Number.isNaN(value)) return 0;
  return Math.max(0, Math.floor(value));
};

const coerceNumber = (value: unknown): number => {
  if (typeof value !== 'number' || Number.isNaN(value)) return 0;
  return Math.max(0, value);
};

const resolveCustomerId = async (req: Request): Promise<string | null> => {
  if (req.user?.userId) return req.user.userId;

  const sdk = (req as Request & { sdk?: SdkContext }).sdk;
  if (!sdk?.publisherId) return null;

  const result = await query<{ id: string }>(
    `SELECT id FROM users WHERE publisher_id = $1 ORDER BY created_at ASC LIMIT 1`,
    [sdk.publisherId]
  );

  return result.rows[0]?.id ?? null;
};

const resolveSubscriptionId = async (customerId: string): Promise<string | null> => {
  const result = await query<{ id: string }>(
    `SELECT id FROM subscriptions
     WHERE customer_id = $1 AND status IN ('trial', 'active', 'past_due')
     ORDER BY updated_at DESC NULLS LAST, created_at DESC
     LIMIT 1`,
    [customerId]
  );

  return result.rows[0]?.id ?? null;
};

export const recordSdkUsage = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const customerId = await resolveCustomerId(req);
    if (!customerId) {
      throw new AppError('Missing customer context', 401);
    }

    const sdk = (req as Request & { sdk?: SdkContext }).sdk;
    const metrics = req.body?.metrics || {};
    const breakdown = req.body?.breakdown || {};

    const impressions = coerceInt(metrics.adImpressions ?? metrics.adRequests ?? 0);
    const apiCalls = coerceInt(metrics.apiCalls ?? 0);
    const dataTransferGb = coerceNumber(metrics.dataTransferGb ?? 0);

    if (impressions === 0 && apiCalls === 0 && dataTransferGb === 0) {
      res.json({
        success: true,
        recorded: false,
        message: 'No billable usage in payload',
      });
      return;
    }

    const metadata = {
      app_id: sdk?.appId,
      sdk_environment: sdk?.environment,
      sdk_key_id: sdk?.keyId,
      placement_count: Object.keys(breakdown.byPlacement || {}).length || undefined,
      adapter_count: Object.keys(breakdown.byAdapter || {}).length || undefined,
      format_count: Object.keys(breakdown.byAdFormat || {}).length || undefined,
    };

    const recordPromises: Array<Promise<void>> = [];
    if (impressions > 0) {
      recordPromises.push(
        usageMeteringService.recordUsage(customerId, 'impressions', impressions, metadata)
      );
    }
    if (apiCalls > 0) {
      recordPromises.push(
        usageMeteringService.recordUsage(customerId, 'api_calls', apiCalls, metadata)
      );
    }
    if (dataTransferGb > 0) {
      recordPromises.push(
        usageMeteringService.recordUsage(
          customerId,
          'data_transfer',
          Math.ceil(dataTransferGb),
          metadata
        )
      );
    }

    await Promise.all(recordPromises);

    const subscriptionId = await resolveSubscriptionId(customerId);
    if (subscriptionId) {
      const now = new Date();
      const auditPromises: Array<Promise<string>> = [];
      if (impressions > 0) {
        auditPromises.push(
          billingAuditTrailService.logUsageMetering({
            customerId,
            subscriptionId,
            metric: 'impressions',
            quantity: impressions,
            timestamp: now,
            metadata,
          })
        );
      }
      if (apiCalls > 0) {
        auditPromises.push(
          billingAuditTrailService.logUsageMetering({
            customerId,
            subscriptionId,
            metric: 'api_calls',
            quantity: apiCalls,
            timestamp: now,
            metadata,
          })
        );
      }
      if (dataTransferGb > 0) {
        auditPromises.push(
          billingAuditTrailService.logUsageMetering({
            customerId,
            subscriptionId,
            metric: 'data_transfer',
            quantity: Math.ceil(dataTransferGb),
            timestamp: now,
            metadata,
          })
        );
      }
      await Promise.all(auditPromises);
    }

    res.json({
      success: true,
      recorded: true,
      data: {
        impressions,
        api_calls: apiCalls,
        data_transfer_gb: dataTransferGb,
      },
    });
  } catch (error) {
    logger.error('Error recording SDK usage', { error });
    next(error);
  }
};
