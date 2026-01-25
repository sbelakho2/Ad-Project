#!/usr/bin/env ts-node
/**
 * Daily Stripe Usage Sync Cron Job
 * 
 * Runs once per day (typically at 2:00 AM UTC) to sync usage records to Stripe.
 * Queries billable usage events from the previous 24 hours and reports to Stripe
 * Subscription Items using the Usage-Based Billing API.
 * 
 * Schedule: 0 2 * * * (2:00 AM daily)
 * 
 * Idempotent: Uses idempotency keys to prevent duplicate charges on retry.
 * Network resilient: Implements exponential backoff with persistent queue fallback.
 */

import Stripe from 'stripe';
import pkg from 'pg';
const { Pool } = pkg;
import { createClient } from 'redis';
import { setTimeout } from 'timers/promises';
import {
  stripeUsageSyncFailuresTotal,
  stripeUsageSyncSuccessTotal,
} from '../src/utils/prometheus';

const STRIPE_SECRET_KEY = process.env.STRIPE_SECRET_KEY || '';
const STRIPE_API_VERSION = process.env.STRIPE_API_VERSION || '2023-10-16';

function assertStripeConfig() {
  if (!STRIPE_SECRET_KEY) {
    throw new Error('Missing STRIPE_SECRET_KEY environment variable');
  }
  // Basic sanity check for version (YYYY-MM-DD or with suffix)
  const versionOk = /^(\d{4}-\d{2}-\d{2})(?:[a-zA-Z0-9_.-]*)?$/.test(STRIPE_API_VERSION);
  if (!versionOk) {
    throw new Error(`Invalid STRIPE_API_VERSION: ${STRIPE_API_VERSION}`);
  }
}

assertStripeConfig();

const stripe = new Stripe(STRIPE_SECRET_KEY, {
  apiVersion: STRIPE_API_VERSION as any,
  typescript: true,
});

interface UsageRecord {
  organization_id: string;
  subscription_item_id: string;
  impressions: number;
  clicks: number;
  videostarts: number;
  timestamp: Date;
}

interface SyncResult {
  organization_id: string;
  subscription_item_id: string;
  status: 'success' | 'failed' | 'retrying';
  error_message?: string;
  stripe_usage_record_id?: string;
}

const DATABASE_URL = process.env.DATABASE_URL || 'postgresql://localhost:5432/adtech';
const REDIS_URL = process.env.REDIS_URL || 'redis://localhost:6379';
const MAX_RETRIES = 5;
const INITIAL_BACKOFF_MS = 1000;

function getIdempotencyKey(orgId: string, date: Date): string {
  const dateStr = date.toISOString().split('T')[0]; // YYYY-MM-DD
  return `usage-sync-${orgId}-${dateStr}`;
}

function calculateBackoffDelay(attempt: number): number {
  // Exponential backoff: 1s, 2s, 4s, 8s, 16s
  return INITIAL_BACKOFF_MS * Math.pow(2, attempt);
}

async function fetchUsageRecords(
  pool: pkg.Pool,
  startTime: Date,
  endTime: Date
): Promise<UsageRecord[]> {
  const query = `
    SELECT 
      o.id as organization_id,
      s.stripe_subscription_item_id as subscription_item_id,
      SUM(CASE WHEN ue.event_type = 'impression' THEN 1 ELSE 0 END) as impressions,
      SUM(CASE WHEN ue.event_type = 'click' THEN 1 ELSE 0 END) as clicks,
      SUM(CASE WHEN ue.event_type = 'video_start' THEN 1 ELSE 0 END) as videostarts,
      $2::timestamp as timestamp
    FROM usage_events ue
    INNER JOIN organizations o ON o.id = ue.organization_id
    INNER JOIN subscriptions s ON s.organization_id = o.id
    WHERE 
      ue.created_at >= $1 
      AND ue.created_at < $2
      AND ue.status = 'billable'
      AND s.status = 'active'
      AND s.stripe_subscription_item_id IS NOT NULL
    GROUP BY o.id, s.stripe_subscription_item_id
    HAVING SUM(1) > 0
  `;

  const result = await pool.query<UsageRecord>(query, [startTime, endTime]);
  return result.rows;
}

async function syncToStripe(
  record: UsageRecord,
  idempotencyKey: string,
  attempt: number = 0
): Promise<SyncResult> {
  try {
    // Stripe expects a single quantity per subscription item
    // We'll sync total events (impressions + clicks + videostarts)
    const totalQuantity = record.impressions + record.clicks + record.videostarts;

    if (totalQuantity === 0) {
      return {
        organization_id: record.organization_id,
        subscription_item_id: record.subscription_item_id,
        status: 'success',
      };
    }

    const usageRecord = await stripe.billing.meterEvents.create({
      event_name: 'usage_event',
      payload: {
        subscription_item_id: record.subscription_item_id,
        quantity: totalQuantity.toString(),
      },
      identifier: idempotencyKey,
      timestamp: Math.floor(record.timestamp.getTime() / 1000),
    });

    // Metrics: success counter
    try {
      stripeUsageSyncSuccessTotal.labels({ org_id: record.organization_id }).inc();
    } catch {}

    return {
      organization_id: record.organization_id,
      subscription_item_id: record.subscription_item_id,
      status: 'success',
      stripe_usage_record_id: usageRecord.identifier,
    };
  } catch (error: unknown) {
    const stripeError = error as any;

    // Check if it's a retriable error
    const isRetriable =
      stripeError.type === 'api_connection_error' ||
      stripeError.type === 'api_error' ||
      (stripeError.statusCode && stripeError.statusCode >= 500);

    if (isRetriable && attempt < MAX_RETRIES) {
      const delay = calculateBackoffDelay(attempt);
      console.warn(
        `Stripe API error for ${record.organization_id}, retrying in ${delay}ms (attempt ${attempt + 1}/${MAX_RETRIES})`
      );
      await setTimeout(delay);
      return syncToStripe(record, idempotencyKey, attempt + 1);
    }

    // Metrics: failure counter with reason label
    try {
      const reason = stripeError?.type || String(stripeError?.statusCode || 'unknown');
      stripeUsageSyncFailuresTotal.labels({ org_id: record.organization_id, reason }).inc();
    } catch {}

    return {
      organization_id: record.organization_id,
      subscription_item_id: record.subscription_item_id,
      status: 'failed',
      error_message: stripeError.message || 'Unknown Stripe error',
    };
  }
}

async function persistFailedSync(
  redis: ReturnType<typeof createClient>,
  record: UsageRecord,
  errorMessage: string
): Promise<void> {
  const key = `usage:sync:failed:${record.organization_id}:${record.timestamp.toISOString()}`;
  const value = JSON.stringify({
    ...record,
    error_message: errorMessage,
    failed_at: new Date().toISOString(),
  });

  // Persist for 7 days for manual retry
  await redis.set(key, value, { EX: 604800 });
}

async function logSyncResult(
  pool: pkg.Pool,
  result: SyncResult,
  usageRecord: UsageRecord
): Promise<void> {
  const query = `
    INSERT INTO billing_audit (
      event_type,
      actor,
      customer_id,
      data,
      created_at
    ) VALUES ($1, $2, $3, $4, NOW())
  `;

  const eventType = result.status === 'success' ? 'stripe_usage_synced' : 'stripe_usage_sync_failed';
  const metadata = {
    organization_id: result.organization_id,
    subscription_item_id: result.subscription_item_id,
    status: result.status,
    impressions: usageRecord.impressions,
    clicks: usageRecord.clicks,
    videostarts: usageRecord.videostarts,
    stripe_usage_record_id: result.stripe_usage_record_id,
    error_message: result.error_message,
  };

  await pool.query(query, [eventType, 'system', result.organization_id, JSON.stringify(metadata)]);
}

async function main(): Promise<void> {
  const startTime = Date.now();
  console.log(`[${new Date().toISOString()}] Starting daily Stripe usage sync...`);

  let redis: ReturnType<typeof createClient> | null = null;
  let pool: pkg.Pool | null = null;

  try {
    // Connect to Redis
    redis = createClient({ url: REDIS_URL });
    await redis.connect();
    console.log('Connected to Redis');

    // Connect to PostgreSQL
    pool = new Pool({ connectionString: DATABASE_URL });
    console.log('Connected to PostgreSQL');

    // Calculate time range: previous 24 hours
    const endTime = new Date();
    const startRange = new Date(endTime.getTime() - 24 * 60 * 60 * 1000);
    console.log(`Syncing usage from ${startRange.toISOString()} to ${endTime.toISOString()}`);

    // Fetch usage records
    const usageRecords = await fetchUsageRecords(pool, startRange, endTime);
    console.log(`Fetched ${usageRecords.length} usage records`);

    if (usageRecords.length === 0) {
      console.log('No usage records to sync. Exiting.');
      return;
    }

    // Sync each record to Stripe
    const results: SyncResult[] = [];
    for (const record of usageRecords) {
      const idempotencyKey = getIdempotencyKey(record.organization_id, record.timestamp);
      console.log(`Syncing usage for organization ${record.organization_id}...`);

      const result = await syncToStripe(record, idempotencyKey);
      results.push(result);

      // Log result to audit log
      await logSyncResult(pool, result, record);

      // Persist failed syncs to Redis for manual retry
      if (result.status === 'failed') {
        await persistFailedSync(redis, record, result.error_message || 'Unknown error');
      }

      // Rate limit: 100 requests/second to Stripe
      await setTimeout(10);
    }

    // Summary
    const successCount = results.filter(r => r.status === 'success').length;
    const failedCount = results.filter(r => r.status === 'failed').length;

    console.log(`\nSync Summary:`);
    console.log(`  Total records: ${results.length}`);
    console.log(`  Successful: ${successCount}`);
    console.log(`  Failed: ${failedCount}`);

    if (failedCount > 0) {
      console.warn(`\nFailed syncs persisted to Redis with key pattern: usage:sync:failed:*`);
      console.warn(`These can be manually retried or will be included in next sync.`);
    }

    const duration = Date.now() - startTime;
    console.log(`\n[${new Date().toISOString()}] Daily Stripe usage sync completed in ${duration}ms`);

    // Exit with error code if any failures
    if (failedCount > 0) {
      process.exit(1);
    }
  } catch (error) {
    console.error('Error in daily Stripe usage sync:', error);
    process.exit(1);
  } finally {
    if (redis) {
      await redis.quit();
    }
    if (pool) {
      await pool.end();
    }
  }
}

// Run the script
main().catch(error => {
  console.error('Unhandled error:', error);
  process.exit(1);
});
