// services/billing/UsageMeteringService.ts
// Tracks SDK usage (impressions, API calls, data transfer) and syncs to Stripe for billing

import type { QueryResultRow } from 'pg';
import Stripe from 'stripe';
import logger from '../../src/utils/logger';
import { query as pgQuery } from '../../src/utils/postgres';
import {
  PLATFORM_TIER_ORDER,
  PLATFORM_TIER_USAGE_LIMITS,
  type PlatformTierId,
  type PlatformTierUsageLimits,
  resolvePlatformTierId,
} from '../../src/config/platformTiers';

let stripeClient: Stripe | null = null;

const getStripeClient = (): Stripe => {
  if (stripeClient) {
    return stripeClient;
  }

  const secretKey = process.env.STRIPE_SECRET_KEY;
  if (!secretKey) {
    throw new Error('Stripe secret key not configured');
  }

  stripeClient = new Stripe(secretKey, {
    apiVersion: '2025-10-29.clover',
  });

  return stripeClient;
};

const primaryQuery = <T extends QueryResultRow = QueryResultRow>(
  text: string,
  params: ReadonlyArray<unknown> | undefined,
  label: string
) => pgQuery<T>(text, params, { label });

const replicaQuery = <T extends QueryResultRow = QueryResultRow>(
  text: string,
  params: ReadonlyArray<unknown> | undefined,
  label: string
) => pgQuery<T>(text, params, { label, replica: true });

interface UsageMetrics {
  customer_id: string;
  impressions: number;
  api_calls: number;
  data_transfer_gb: number;
  period_start: Date;
  period_end: Date;
}

interface PlanLimits extends PlatformTierUsageLimits {
  plan_type: PlatformTierId;
}

const buildPlanLimits = (
  tier: PlatformTierId,
  overrides?: Partial<PlatformTierUsageLimits>
): PlanLimits => {
  const defaults = PLATFORM_TIER_USAGE_LIMITS[tier];
  const plan: PlanLimits = {
    plan_type: tier,
    included_impressions: Number(overrides?.included_impressions ?? defaults.included_impressions),
    included_api_calls: Number(overrides?.included_api_calls ?? defaults.included_api_calls),
    included_data_transfer_gb: Number(
      overrides?.included_data_transfer_gb ?? defaults.included_data_transfer_gb
    ),
    overage_price_impressions_cents: Number(
      overrides?.overage_price_impressions_cents ?? defaults.overage_price_impressions_cents
    ),
    overage_price_api_calls_cents: Number(
      overrides?.overage_price_api_calls_cents ?? defaults.overage_price_api_calls_cents
    ),
    overage_price_data_transfer_cents: Number(
      overrides?.overage_price_data_transfer_cents ?? defaults.overage_price_data_transfer_cents
    ),
  };

  const hasInvalidValue = [
    plan.included_impressions,
    plan.included_api_calls,
    plan.included_data_transfer_gb,
    plan.overage_price_impressions_cents,
    plan.overage_price_api_calls_cents,
    plan.overage_price_data_transfer_cents,
  ].some((value) => Number.isNaN(value) || value < 0);

  if (hasInvalidValue) {
    logger.warn('[UsageMetering] Invalid overrides detected for plan; reverting to defaults', {
      tier,
    });
    return { plan_type: tier, ...defaults };
  }

  return plan;
};

const createDefaultPlanLimits = (): Record<PlatformTierId, PlanLimits> =>
  PLATFORM_TIER_ORDER.reduce<Record<PlatformTierId, PlanLimits>>((acc, tier) => {
    acc[tier] = buildPlanLimits(tier);
    return acc;
  }, {} as Record<PlatformTierId, PlanLimits>);

function resolvePlanLimitsFromEnv(): Record<PlatformTierId, PlanLimits> {
  const base = createDefaultPlanLimits();
  const raw = process.env.BILLING_PLAN_LIMITS_JSON;
  if (!raw) return base;

  try {
    const parsed = JSON.parse(raw) as Record<string, Partial<PlatformTierUsageLimits>>;
    for (const [key, overrides] of Object.entries(parsed)) {
      const tier = resolvePlatformTierId(key);
      if (!tier) {
        logger.warn('[UsageMetering] Unknown plan in BILLING_PLAN_LIMITS_JSON; skipping', {
          plan: key,
        });
        continue;
      }

      base[tier] = buildPlanLimits(tier, overrides);
    }

    return base;
  } catch (e) {
    logger.warn('[UsageMetering] Failed to parse BILLING_PLAN_LIMITS_JSON; using defaults', {
      error: (e as Error).message,
    });
    return base;
  }
}

const coercePlanType = (value: string): PlatformTierId => {
  const tier = resolvePlatformTierId(value);
  if (!tier) {
    throw new Error(`Unknown plan type: ${value}`);
  }
  return tier;
};

export class UsageMeteringService {
  private planLimits: Record<PlatformTierId, PlanLimits>;

  constructor(planLimits?: Record<PlatformTierId, PlanLimits>) {
    this.planLimits = planLimits ?? resolvePlanLimitsFromEnv();
  }

  /**
   * Record usage event (called by SDK telemetry endpoint)
   */
  async recordUsage(
    customerId: string,
    metricType: 'impressions' | 'api_calls' | 'data_transfer',
    quantity: number,
    metadata?: Record<string, unknown>
  ): Promise<void> {
    const metadataJson = JSON.stringify(metadata || {});

    await primaryQuery(
      `INSERT INTO usage_records (customer_id, metric_type, quantity, metadata, recorded_at)
       VALUES ($1, $2, $3, $4, NOW())`,
      [customerId, metricType, quantity, metadataJson],
      'USAGE_RECORD_INSERT'
    );
  }

  /**
   * Get current billing period usage for a customer
   */
  async getCurrentPeriodUsage(customerId: string): Promise<UsageMetrics> {
    const subscription = await primaryQuery(
      `SELECT current_period_start, current_period_end, plan_type
       FROM subscriptions
       WHERE customer_id = $1 AND status = 'active'`,
      [customerId],
      'USAGE_SUBSCRIPTION_WINDOW'
    );

    if (subscription.rows.length === 0) {
      // Return empty usage if no active subscription
      return {
        customer_id: customerId,
        period_start: new Date(),
        period_end: new Date(),
        impressions: 0,
        api_calls: 0,
        data_transfer_gb: 0
      };
    }

    const { current_period_start, current_period_end } = subscription.rows[0];

    // Aggregate usage from PostgreSQL
    const usage = await primaryQuery(
      `SELECT 
         metric_type,
         SUM(quantity) as total
       FROM usage_records
       WHERE customer_id = $1
         AND recorded_at >= $2
         AND recorded_at <= $3
       GROUP BY metric_type`,
      [customerId, current_period_start, current_period_end],
      'USAGE_PERIOD_AGGREGATE'
    );

    const metrics: UsageMetrics = {
      customer_id: customerId,
      impressions: 0,
      api_calls: 0,
      data_transfer_gb: 0,
      period_start: current_period_start,
      period_end: current_period_end,
    };

    for (const row of usage.rows) {
      if (row.metric_type === 'impressions') {
        metrics.impressions = parseInt(row.total);
      } else if (row.metric_type === 'api_calls') {
        metrics.api_calls = parseInt(row.total);
      } else if (row.metric_type === 'data_transfer') {
        metrics.data_transfer_gb = parseFloat(row.total);
      }
    }

    return metrics;
  }

  /**
   * Calculate overage charges for current period
   */
  async calculateOverages(customerId: string): Promise<{
    impressions_overage: number;
    impressions_overage_cost_cents: number;
    api_calls_overage: number;
    api_calls_overage_cost_cents: number;
    data_transfer_overage_gb: number;
    data_transfer_overage_cost_cents: number;
    total_overage_cost_cents: number;
  }> {
    const subscription = await primaryQuery(
      `SELECT plan_type FROM subscriptions WHERE customer_id = $1 AND status = 'active'`,
      [customerId],
      'USAGE_SUBSCRIPTION_PLAN'
    );

    if (subscription.rows.length === 0) {
      // Return zero overages if no active subscription
      return {
        impressions_overage: 0,
        impressions_overage_cost_cents: 0,
        api_calls_overage: 0,
        api_calls_overage_cost_cents: 0,
        data_transfer_overage_gb: 0,
        data_transfer_overage_cost_cents: 0,
        total_overage_cost_cents: 0
      };
    }

    const planType = coercePlanType(subscription.rows[0].plan_type);
    const limits = this.planLimits[planType];

    if (!limits) {
      throw new Error(`Unknown plan type: ${planType}`);
    }

    const usage = await this.getCurrentPeriodUsage(customerId);

    // Calculate overages
    const impressions_overage = Math.max(0, usage.impressions - limits.included_impressions);
    const api_calls_overage = Math.max(0, usage.api_calls - limits.included_api_calls);
    const data_transfer_overage_gb = Math.max(
      0,
      usage.data_transfer_gb - limits.included_data_transfer_gb
    );

    // Calculate costs (impressions and API calls are priced per 1000)
    const impressions_overage_cost_cents =
      Math.ceil(impressions_overage / 1000) * limits.overage_price_impressions_cents;
    const api_calls_overage_cost_cents =
      Math.ceil(api_calls_overage / 1000) * limits.overage_price_api_calls_cents;
    const data_transfer_overage_cost_cents =
      Math.ceil(data_transfer_overage_gb) * limits.overage_price_data_transfer_cents;

    const total_overage_cost_cents =
      impressions_overage_cost_cents +
      api_calls_overage_cost_cents +
      data_transfer_overage_cost_cents;

    return {
      impressions_overage,
      impressions_overage_cost_cents,
      api_calls_overage,
      api_calls_overage_cost_cents,
      data_transfer_overage_gb,
      data_transfer_overage_cost_cents,
      total_overage_cost_cents,
    };
  }

  /**
   * Sync usage to Stripe for metered billing
   * Run this daily via cron
   */
  async syncUsageToStripe(): Promise<void> {
    logger.info('[UsageMetering] Starting daily Stripe usage sync...');

    // Get all active subscriptions
    const subscriptions = await primaryQuery(
      `SELECT 
         s.id,
         s.customer_id,
         s.stripe_subscription_id,
         s.current_period_start,
         s.current_period_end,
         s.plan_type
       FROM subscriptions s
       WHERE s.status = 'active'
         AND s.stripe_subscription_id IS NOT NULL`,
      undefined,
      'USAGE_STRIPE_SUBSCRIPTIONS'
    );

    for (const sub of subscriptions.rows) {
      try {
        const stripe = getStripeClient();

        // Get yesterday's usage (idempotent - Stripe handles duplicates)
        const yesterday = new Date();
        yesterday.setDate(yesterday.getDate() - 1);
        yesterday.setHours(0, 0, 0, 0);

        const yesterdayEnd = new Date(yesterday);
        yesterdayEnd.setHours(23, 59, 59, 999);

        const usage = await primaryQuery(
          `SELECT 
             metric_type,
             SUM(quantity) as total
           FROM usage_records
           WHERE customer_id = $1
             AND recorded_at >= $2
             AND recorded_at <= $3
           GROUP BY metric_type`,
          [sub.customer_id, yesterday, yesterdayEnd],
          'USAGE_STRIPE_YESTERDAY'
        );

        // Get Stripe subscription items
        const stripeSubscription = await stripe.subscriptions.retrieve(
          sub.stripe_subscription_id,
          { expand: ['items'] }
        );

        // Find metered items (these have usage_type: 'metered' in Stripe)
        for (const item of stripeSubscription.items.data) {
          const price = item.price;
          if (!price.recurring?.usage_type || price.recurring.usage_type !== 'metered') {
            continue;
          }

          // Match metric type from price metadata
          const metricType = price.metadata?.metric_type;
          if (!metricType) continue;

          const metricUsage = usage.rows.find((r) => r.metric_type === metricType);
          if (!metricUsage) continue;

          const quantity = parseInt(metricUsage.total);

          // Report usage to Stripe (usage records API)
          await (stripe.billing.meterEvents as any).create({
            event_name: metricType,
            payload: {
              value: quantity.toString(),
              stripe_customer_id: sub.customer_id,
            },
            timestamp: Math.floor(yesterday.getTime() / 1000),
          });

          logger.info('[UsageMetering] Synced usage to Stripe', {
            customerId: sub.customer_id,
            metricType,
            quantity,
          });
        }
      } catch (error) {
        logger.error('[UsageMetering] Error syncing usage for customer', {
          customerId: sub.customer_id,
          error,
        });
        // Continue with other customers
      }
    }

    logger.info('[UsageMetering] Stripe usage sync complete');
  }

  /**
   * Check for customers approaching limits and send alerts
   * Run this daily via cron
   */
  async checkUsageLimits(): Promise<void> {
    logger.info('[UsageMetering] Checking usage limits...');

    const subscriptions = await primaryQuery(
      `SELECT customer_id, plan_type, included_impressions
       FROM subscriptions
       WHERE status = 'active'`,
      undefined,
      'USAGE_LIMITS_SUBSCRIPTIONS'
    );

    for (const sub of subscriptions.rows) {
      try {
        const usage = await this.getCurrentPeriodUsage(sub.customer_id);
        const planType = coercePlanType(sub.plan_type);
        const limits = this.planLimits[planType];

        if (!limits) continue;

        const impressionsPct = (usage.impressions / limits.included_impressions) * 100;

        // Send alerts at 80%, 90%, 100%, 110%
        if (impressionsPct >= 80 && impressionsPct < 90) {
          await this.sendUsageAlert(sub.customer_id, 80, usage.impressions, limits);
        } else if (impressionsPct >= 90 && impressionsPct < 100) {
          await this.sendUsageAlert(sub.customer_id, 90, usage.impressions, limits);
        } else if (impressionsPct >= 100 && impressionsPct < 110) {
          await this.sendUsageAlert(sub.customer_id, 100, usage.impressions, limits);
        } else if (impressionsPct >= 110) {
          await this.sendUsageAlert(sub.customer_id, 110, usage.impressions, limits);
        }
      } catch (error) {
        logger.error('[UsageMetering] Error checking usage limits for customer', {
          customerId: sub.customer_id,
          error,
        });
      }
    }

    logger.info('[UsageMetering] Usage limit checks complete');
  }

  /**
   * Send usage alert email
   */
  private async sendUsageAlert(
    customerId: string,
    percentUsed: number,
    currentUsage: number,
    limits: PlanLimits
  ): Promise<void> {
    // Check if we've already sent this alert
    const existingAlert = await primaryQuery(
      `SELECT id FROM usage_alerts
       WHERE customer_id = $1
         AND alert_type = $2
         AND created_at >= NOW() - INTERVAL '24 hours'`,
      [customerId, `usage_${percentUsed}`],
      'USAGE_ALERT_EXISTING'
    );

    if (existingAlert.rows.length > 0) {
      return; // Already sent this alert today
    }

    // Record alert
    await primaryQuery(
      `INSERT INTO usage_alerts (customer_id, alert_type, usage_amount, limit_amount, created_at)
       VALUES ($1, $2, $3, $4, NOW())`,
      [customerId, `usage_${percentUsed}`, currentUsage, limits.included_impressions],
      'USAGE_ALERT_INSERT'
    );

    // Get user email
    const user = await primaryQuery(
      `SELECT email FROM users WHERE id = $1`,
      [customerId],
      'USAGE_ALERT_USER_EMAIL'
    );

    if (user.rows.length === 0) return;

    const email = user.rows[0].email;

    // Email service integration via event emission
    logger.info('[UsageMetering] Usage alert threshold reached', {
      customerId,
      email,
      percentUsed,
    });

    // Emit event (to be handled by EmailService)
    await this.emitEvent('usage.alert', {
      customer_id: customerId,
      email: email,
      percent_used: percentUsed,
      current_usage: currentUsage,
      limit: limits.included_impressions,
      plan_type: limits.plan_type,
    });
  }

  /**
   * Get subscription details for a customer
   */
  async getSubscriptionDetails(customerId: string): Promise<{
    plan_type: string;
    included_impressions: number;
    included_api_calls: number;
    included_data_transfer_gb: number;
  }> {
    const subscription = await primaryQuery(
      `SELECT plan_type, included_impressions
       FROM subscriptions
       WHERE customer_id = $1 AND status = 'active'`,
      [customerId],
      'USAGE_SUBSCRIPTION_DETAILS'
    );

    if (subscription.rows.length === 0) {
      // Return default starter tier limits if no active subscription
      const starterLimits = this.planLimits.starter;
      
      return {
        plan_type: 'starter',
        included_impressions: starterLimits.included_impressions,
        included_api_calls: starterLimits.included_api_calls,
        included_data_transfer_gb: starterLimits.included_data_transfer_gb,
      };
    }

    const planType = coercePlanType(subscription.rows[0].plan_type);
    const limits = this.planLimits[planType];

    if (!limits) {
      throw new Error(`Unknown plan type: ${planType}`);
    }

    return {
      plan_type: planType,
      included_impressions: limits.included_impressions,
      included_api_calls: limits.included_api_calls,
      included_data_transfer_gb: limits.included_data_transfer_gb,
    };
  }

  /**
   * Get usage analytics for customer dashboard
   */
  async getUsageAnalytics(
    customerId: string,
    days: number = 30
  ): Promise<{
    daily_usage: Array<{ date: string; impressions: number; api_calls: number }>;
    total_impressions: number;
    total_api_calls: number;
    avg_daily_impressions: number;
  }> {
    const windowDays =
      Number.isFinite(days) && days > 0 ? Math.max(1, Math.min(Math.floor(days), 180)) : 30;
    const startDate = new Date();
    startDate.setHours(0, 0, 0, 0);
    startDate.setDate(startDate.getDate() - windowDays);

    type UsageAnalyticsRow = {
      date: string;
      metric_type: 'impressions' | 'api_calls' | 'data_transfer';
      total: string | number;
    };

    const analytics = await replicaQuery<UsageAnalyticsRow>(
      `SELECT 
         to_char(date_trunc('day', recorded_at), 'YYYY-MM-DD') AS date,
         metric_type,
         SUM(quantity)::numeric AS total
       FROM usage_records
       WHERE customer_id = $1
         AND recorded_at >= $2
       GROUP BY date, metric_type
       ORDER BY date ASC`,
      [customerId, startDate],
      'USAGE_ANALYTICS_DAILY'
    );

    const dailyUsageMap = new Map<string, { date: string; impressions: number; api_calls: number }>();

    for (const row of analytics.rows) {
      const dateStr = row.date;
      const totalValue = Number(row.total) || 0;
      const current = dailyUsageMap.get(dateStr) ?? {
        date: dateStr,
        impressions: 0,
        api_calls: 0,
      };
      const nextValue = { ...current };

      if (row.metric_type === 'impressions') {
        nextValue.impressions = totalValue;
      } else if (row.metric_type === 'api_calls') {
        nextValue.api_calls = totalValue;
      }

      dailyUsageMap.set(dateStr, nextValue);
    }

    const daily_usage = Array.from(dailyUsageMap.values()).sort((a, b) => a.date.localeCompare(b.date));

    const total_impressions = daily_usage.reduce((sum, day) => sum + day.impressions, 0);
    const total_api_calls = daily_usage.reduce((sum, day) => sum + day.api_calls, 0);
    const avg_daily_impressions = windowDays > 0 ? total_impressions / windowDays : 0;

    return {
      daily_usage,
      total_impressions,
      total_api_calls,
      avg_daily_impressions,
    };
  }

  /**
   * Emit event to event bus (for email service, webhooks, etc.)
   */
  private async emitEvent(eventType: string, data: any): Promise<void> {
    // Simple implementation: insert into events table
    // In production, use Redis Pub/Sub, Kafka, or RabbitMQ
    await primaryQuery(
      `INSERT INTO events (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      [eventType, JSON.stringify(data)],
      'USAGE_EVENT_EMIT'
    );
  }
}

export const usageMeteringService = new UsageMeteringService();
