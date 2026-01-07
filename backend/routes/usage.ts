// routes/usage.ts
// API routes for usage tracking and metering

import express, { Request } from 'express';
import { usageMeteringService } from '../services/billing/UsageMeteringService';
import { authenticate } from '../middleware/auth';
import { assertPlatformTierId } from '../src/config/platformTiers';

/** Authenticated request with user context */
interface AuthenticatedRequest extends Request {
  user: {
    id: string;
    email?: string;
    role?: string;
  };
}

const router = express.Router();

/**
 * POST /api/usage/track
 * Record usage event from SDK telemetry
 * 
 * Body:
 * {
 *   "api_key": "rsk_...",
 *   "metric_type": "impressions",
 *   "quantity": 1000,
 *   "metadata": { "platform": "iOS", "sdk_version": "1.0.0" }
 * }
 */
router.post('/track', async (req, res) => {
  try {
    const { api_key, metric_type, quantity, metadata } = req.body;

    if (!api_key || !metric_type || quantity === undefined) {
      return res.status(400).json({
        error: 'Missing required fields: api_key, metric_type, quantity',
      });
    }

    // Validate metric type
    const validMetrics = ['impressions', 'api_calls', 'data_transfer'];
    if (!validMetrics.includes(metric_type)) {
      return res.status(400).json({
        error: `Invalid metric_type. Must be one of: ${validMetrics.join(', ')}`,
      });
    }

    // Verify API key and get customer_id
    const { Pool } = require('pg');
    const db = new Pool({ connectionString: process.env.DATABASE_URL });

    const result = await db.query(
      `SELECT user_id, status FROM api_keys WHERE key = $1`,
      [api_key]
    );

    if (result.rows.length === 0) {
      return res.status(401).json({ error: 'Invalid API key' });
    }

    const apiKey = result.rows[0];

    if (apiKey.status !== 'active') {
      return res.status(403).json({
        error: `API key is ${apiKey.status}`,
        status: apiKey.status,
      });
    }

    // Record usage
    await usageMeteringService.recordUsage(
      apiKey.user_id,
      metric_type,
      quantity,
      metadata || {}
    );

    res.json({
      success: true,
      recorded: {
        customer_id: apiKey.user_id,
        metric_type,
        quantity,
      },
    });
  } catch (error) {
    console.error('[Usage] Error tracking usage:', error);
    res.status(500).json({ error: 'Failed to record usage' });
  }
});

/**
 * GET /api/usage/current
 * Get current billing period usage for authenticated user
 */
router.get('/current', authenticate, async (req, res) => {
  try {
    const customerId = (req as AuthenticatedRequest).user.id;

    const usage = await usageMeteringService.getCurrentPeriodUsage(customerId);
    const overages = await usageMeteringService.calculateOverages(customerId);

    res.json({
      usage,
      overages,
    });
  } catch (error) {
    console.error('[Usage] Error getting current usage:', error);
    res.status(500).json({ error: 'Failed to retrieve usage data' });
  }
});

/**
 * GET /api/usage/analytics
 * Get usage analytics for dashboard (last 30 days by default)
 */
router.get('/analytics', authenticate, async (req, res) => {
  try {
    const customerId = (req as AuthenticatedRequest).user.id;
    const days = parseInt(req.query.days as string) || 30;

    if (days < 1 || days > 365) {
      return res.status(400).json({ error: 'days must be between 1 and 365' });
    }

    const analytics = await usageMeteringService.getUsageAnalytics(customerId, days);

    res.json(analytics);
  } catch (error) {
    console.error('[Usage] Error getting usage analytics:', error);
    res.status(500).json({ error: 'Failed to retrieve analytics' });
  }
});

/**
 * GET /api/usage/limits
 * Get plan limits and current usage percentage
 */
router.get('/limits', authenticate, async (req, res) => {
  try {
    const customerId = (req as AuthenticatedRequest).user.id;

    const { Pool } = require('pg');
    const db = new Pool({ connectionString: process.env.DATABASE_URL });

    const subscription = await db.query(
      `SELECT plan_type, included_impressions FROM subscriptions WHERE customer_id = $1 AND status = 'active'`,
      [customerId]
    );

    if (subscription.rows.length === 0) {
      return res.status(404).json({ error: 'No active subscription found' });
    }

    const planType = assertPlatformTierId(subscription.rows[0].plan_type, 'Unknown platform tier on subscription');
    const { included_impressions } = subscription.rows[0];
    const usage = await usageMeteringService.getCurrentPeriodUsage(customerId);

    const percentUsed = (usage.impressions / included_impressions) * 100;
    const remaining = Math.max(0, included_impressions - usage.impressions);

    res.json({
      plan_type: planType,
      limits: {
        impressions: included_impressions,
      },
      current_usage: {
        impressions: usage.impressions,
      },
      percent_used: percentUsed,
      remaining: {
        impressions: remaining,
      },
      will_exceed: percentUsed > 100,
    });
  } catch (error) {
    console.error('[Usage] Error getting usage limits:', error);
    res.status(500).json({ error: 'Failed to retrieve usage limits' });
  }
});

export default router;
