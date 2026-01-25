#!/usr/bin/env ts-node
/**
 * Hourly Usage Limiter Cron Job
 * 
 * Runs every hour to check if organizations have exceeded their subscription limits.
 * Sets Redis flags (usage:limit:exceeded:{orgId}) to enable request throttling.
 * 
 * Schedule: 0 * * * * (top of every hour)
 * 
 * Idempotent: Can be safely re-run multiple times per hour.
 */

import { createClient } from 'redis';
import pkg from 'pg';
const { Pool } = pkg;
import { billingUsageLimitExceededTotal } from '../src/utils/prometheus';

interface UsageMetrics {
  organization_id: string;
  impressions: number;
  clicks: number;
  videostarts: number;
  period_start: Date;
  period_end: Date;
}

interface SubscriptionLimits {
  organization_id: string;
  impressions_limit: number;
  clicks_limit: number;
  videostarts_limit: number;
}

interface LimitExceeded {
  organization_id: string;
  exceeded_metrics: string[];
  impressions_percent: number;
  clicks_percent: number;
  videostarts_percent: number;
}

const REDIS_URL = process.env.REDIS_URL || 'redis://localhost:6379';
const DATABASE_URL = process.env.DATABASE_URL || 'postgresql://localhost:5432/adtech';
const LIMIT_THRESHOLD = parseFloat(process.env.USAGE_LIMIT_THRESHOLD || '100'); // Percentage

async function getCurrentBillingPeriod(): Promise<{ start: Date; end: Date }> {
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 0, 0);
  const end = new Date(now.getFullYear(), now.getMonth() + 1, 0, 23, 59, 59, 999);
  return { start, end };
}

async function fetchUsageMetrics(
  pool: pkg.Pool,
  periodStart: Date,
  periodEnd: Date
): Promise<UsageMetrics[]> {
  const query = `
    SELECT 
      organization_id,
      SUM(CASE WHEN event_type = 'impression' THEN 1 ELSE 0 END) as impressions,
      SUM(CASE WHEN event_type = 'click' THEN 1 ELSE 0 END) as clicks,
      SUM(CASE WHEN event_type = 'video_start' THEN 1 ELSE 0 END) as videostarts,
      $1::timestamp as period_start,
      $2::timestamp as period_end
    FROM usage_events
    WHERE 
      created_at >= $1 
      AND created_at <= $2
      AND status = 'billable'
    GROUP BY organization_id
    HAVING COUNT(*) > 0
  `;

  const result = await pool.query<UsageMetrics>(query, [periodStart, periodEnd]);
  return result.rows;
}

async function fetchSubscriptionLimits(
  pool: pkg.Pool,
  organizationIds: string[]
): Promise<Map<string, SubscriptionLimits>> {
  if (organizationIds.length === 0) {
    return new Map();
  }

  const query = `
    SELECT 
      o.id as organization_id,
      COALESCE(sp.impressions_limit, 0) as impressions_limit,
      COALESCE(sp.clicks_limit, 0) as clicks_limit,
      COALESCE(sp.videostarts_limit, 0) as videostarts_limit
    FROM organizations o
    LEFT JOIN subscriptions s ON s.organization_id = o.id AND s.status = 'active'
    LEFT JOIN subscription_plans sp ON sp.id = s.plan_id
    WHERE o.id = ANY($1::uuid[])
  `;

  const result = await pool.query<SubscriptionLimits>(query, [organizationIds]);
  
  const limitsMap = new Map<string, SubscriptionLimits>();
  result.rows.forEach(row => {
    limitsMap.set(row.organization_id, row);
  });
  
  return limitsMap;
}

function calculateLimitExceeded(
  usage: UsageMetrics,
  limits: SubscriptionLimits
): LimitExceeded | null {
  const exceededMetrics: string[] = [];
  
  const impressionsPercent = limits.impressions_limit > 0 
    ? (usage.impressions / limits.impressions_limit) * 100 
    : 0;
  const clicksPercent = limits.clicks_limit > 0 
    ? (usage.clicks / limits.clicks_limit) * 100 
    : 0;
  const videostartsPercent = limits.videostarts_limit > 0 
    ? (usage.videostarts / limits.videostarts_limit) * 100 
    : 0;

  if (impressionsPercent >= LIMIT_THRESHOLD) {
    exceededMetrics.push('impressions');
  }
  if (clicksPercent >= LIMIT_THRESHOLD) {
    exceededMetrics.push('clicks');
  }
  if (videostartsPercent >= LIMIT_THRESHOLD) {
    exceededMetrics.push('videostarts');
  }

  if (exceededMetrics.length === 0) {
    return null;
  }

  return {
    organization_id: usage.organization_id,
    exceeded_metrics: exceededMetrics,
    impressions_percent: impressionsPercent,
    clicks_percent: clicksPercent,
    videostarts_percent: videostartsPercent,
  };
}

async function setLimitFlags(
  redis: ReturnType<typeof createClient>,
  exceeded: LimitExceeded[]
): Promise<void> {
  const pipeline = redis.multi();

  for (const org of exceeded) {
    const key = `usage:limit:exceeded:${org.organization_id}`;
    const value = JSON.stringify({
      exceeded_metrics: org.exceeded_metrics,
      impressions_percent: org.impressions_percent,
      clicks_percent: org.clicks_percent,
      videostarts_percent: org.videostarts_percent,
      updated_at: new Date().toISOString(),
    });

    // Set flag with 2-hour expiry (in case cron fails)
    pipeline.set(key, value, { EX: 7200 });

    // Increment Prometheus counter for observability
    try {
      billingUsageLimitExceededTotal.labels({ org_id: org.organization_id }).inc();
    } catch {
      // no-op if metrics registry is unavailable
    }
  }

  await pipeline.exec();
}

async function clearLimitFlags(
  redis: ReturnType<typeof createClient>,
  organizationIds: string[]
): Promise<void> {
  if (organizationIds.length === 0) {
    return;
  }

  const keys = organizationIds.map(id => `usage:limit:exceeded:${id}`);
  await redis.del(keys);
}

async function logAudit(
  pool: pkg.Pool,
  exceeded: LimitExceeded[]
): Promise<void> {
  if (exceeded.length === 0) {
    return;
  }

  const query = `
    INSERT INTO billing_audit (
      event_type,
      actor,
      customer_id,
      data,
      created_at
    ) VALUES ($1, $2, $3, $4, NOW())
  `;

  for (const org of exceeded) {
    await pool.query(query, [
      'usage_limit_exceeded',
      'system',
      org.organization_id,
      JSON.stringify({
        organization_id: org.organization_id,
        exceeded_metrics: org.exceeded_metrics,
        impressions_percent: org.impressions_percent.toFixed(2),
        clicks_percent: org.clicks_percent.toFixed(2),
        videostarts_percent: org.videostarts_percent.toFixed(2),
      }),
    ]);
  }
}

async function main(): Promise<void> {
  const startTime = Date.now();
  console.log(`[${new Date().toISOString()}] Starting hourly usage limiter...`);

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

    // Get current billing period
    const period = await getCurrentBillingPeriod();
    console.log(`Billing period: ${period.start.toISOString()} to ${period.end.toISOString()}`);

    // Fetch usage metrics
    const usageMetrics = await fetchUsageMetrics(pool, period.start, period.end);
    console.log(`Fetched usage for ${usageMetrics.length} organizations`);

    if (usageMetrics.length === 0) {
      console.log('No usage data found. Exiting.');
      return;
    }

    // Fetch subscription limits
    const organizationIds = usageMetrics.map(u => u.organization_id);
    const limitsMap = await fetchSubscriptionLimits(pool, organizationIds);
    console.log(`Fetched limits for ${limitsMap.size} organizations`);

    // Calculate exceeded limits
    const exceeded: LimitExceeded[] = [];
    const withinLimits: string[] = [];

    for (const usage of usageMetrics) {
      const limits = limitsMap.get(usage.organization_id);
      if (!limits) {
        console.warn(`No limits found for organization ${usage.organization_id}`);
        continue;
      }

      const result = calculateLimitExceeded(usage, limits);
      if (result) {
        exceeded.push(result);
      } else {
        withinLimits.push(usage.organization_id);
      }
    }

    console.log(`Organizations exceeding limits: ${exceeded.length}`);
    console.log(`Organizations within limits: ${withinLimits.length}`);

    // Set Redis flags for exceeded orgs
    if (exceeded.length > 0) {
      await setLimitFlags(redis, exceeded);
      console.log(`Set limit flags for ${exceeded.length} organizations`);

      // Log audit events
      await logAudit(pool, exceeded);
      console.log(`Logged ${exceeded.length} audit events`);
    }

    // Clear flags for orgs within limits
    if (withinLimits.length > 0) {
      await clearLimitFlags(redis, withinLimits);
      console.log(`Cleared limit flags for ${withinLimits.length} organizations`);
    }

    const duration = Date.now() - startTime;
    console.log(`[${new Date().toISOString()}] Hourly usage limiter completed in ${duration}ms`);
  } catch (error) {
    console.error('Error in hourly usage limiter:', error);
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
