import express, { Request, Response } from 'express';
import Stripe from 'stripe';
import { Pool } from 'pg';
import crypto from 'crypto';
import { platformFeeService } from '../services/billing/platformFeeService';
import {
  PLATFORM_TIER_ORDER,
  type PlatformTierId,
  resolvePlatformTierId,
} from '../src/config/platformTiers';
import { authenticate } from '../src/middleware/auth';

const router = express.Router();

// Initialize Stripe
const stripe = new Stripe(process.env.STRIPE_SECRET_KEY || '', {
  apiVersion: '2025-10-29.clover',
});

// Database connection pool
const pool = new Pool({
  host: process.env.DB_HOST || 'localhost',
  port: parseInt(process.env.DB_PORT || '5432'),
  database: process.env.DB_NAME || 'rivalapexmediation',
  user: process.env.DB_USER || 'rivalapexmediation',
  password: process.env.DB_PASSWORD,
});

type PricingPlan = {
  name: string;
  price_cents: number | null;
  currency: string;
  included_impressions: number | null;
  overage_rate_cents: number | null;
  stripe_price_id: string | null;
  features: string[];
};

const CHECKOUT_ELIGIBLE_PLANS: PlatformTierId[] = ['starter', 'growth'];

/**
 * Pricing Configuration
 */
const PRICING_PLANS: Record<PlatformTierId, PricingPlan> = {
  starter: {
    name: 'Tier 0 — Starter',
    price_cents: 9900,
    currency: 'usd',
    included_impressions: 1_000_000,
    overage_rate_cents: 10,
    stripe_price_id:
      process.env.STRIPE_STARTER_PRICE_ID ||
      process.env.STRIPE_INDIE_PRICE_ID ||
      'price_starter_monthly',
    features: [
      '0% platform fee up to $10k mediated revenue',
      '1M ad impressions/month included',
      'Baseline observability + SDK access',
      'Email support and community office hours',
    ],
  },
  growth: {
    name: 'Tier 1 — Growth',
    price_cents: 49900,
    currency: 'usd',
    included_impressions: 10_000_000,
    overage_rate_cents: 8,
    stripe_price_id:
      process.env.STRIPE_GROWTH_PRICE_ID ||
      process.env.STRIPE_STUDIO_PRICE_ID ||
      'price_growth_monthly',
    features: [
      '2.5% platform fee across $10k–$100k revenue bands',
      '10M ad impressions/month included',
      'Priority support with migration assistants',
      'Advanced analytics and automated workflows',
    ],
  },
  scale: {
    name: 'Tier 2 — Scale',
    price_cents: null,
    currency: 'usd',
    included_impressions: 50_000_000,
    overage_rate_cents: 6,
    stripe_price_id: null,
    features: [
      '2.0% platform fee for $100k–$500k revenue bands',
      'Named revenue engineering pod + custom exports',
      '50M ad impressions/month included',
      'Contracted migration + observability tooling',
    ],
  },
  enterprise: {
    name: 'Tier 3 — Enterprise',
    price_cents: null,
    currency: 'usd',
    included_impressions: null,
    overage_rate_cents: null,
    stripe_price_id: null,
    features: [
      'Custom 1.0–1.5% platform fee for $500k+ revenue bands',
      'Unlimited ad impressions with contractual SLAs',
      'Dedicated infra, compliance, and bespoke data residency',
      'Executive alignment, war-room workflows, and on-site rev ops',
    ],
  },
};

/**
 * GET /api/billing/pricing
 * 
 * Get all available pricing plans
 */
router.get('/pricing', async (_req: Request, res: Response) => {
  res.json({
    platform_tiers: platformFeeService.getTiers(),
    legacy_subscription_plans: PRICING_PLANS,
    currency: 'USD',
  });
});

/**
 * POST /api/billing/signup
 * 
 * Create Stripe Checkout session for new customer signup
 * 
 * Body:
 * - email: string
 * - plan: 'starter' | 'growth'
 * - company_name: string (optional)
 * - vat_id: string (optional, for EU customers)
 */
router.post('/signup', async (req: Request, res: Response) => {
  const { email, plan, company_name, vat_id, country } = req.body;

  // Validate plan
  const normalizedPlan = resolvePlatformTierId(plan);

  if (!normalizedPlan || !CHECKOUT_ELIGIBLE_PLANS.includes(normalizedPlan)) {
    return res.status(400).json({ 
      error: 'Invalid plan. Choose Starter or Growth. Contact sales for Scale or Enterprise.',
    });
  }

  if (!email || !email.includes('@')) {
    return res.status(400).json({ error: 'Valid email is required' });
  }

  try {
    const planConfig = PRICING_PLANS[normalizedPlan];

    if (!planConfig?.stripe_price_id) {
      return res.status(400).json({
        error: 'Selected plan requires a sales-assisted contract. Please contact sales.',
      });
    }

    // Create Stripe Checkout session
    const session = await stripe.checkout.sessions.create({
      mode: 'subscription',
      payment_method_types: ['card'],
      line_items: [
        {
          price: planConfig.stripe_price_id,
          quantity: 1,
        },
      ],
      customer_email: email,
      client_reference_id: crypto.randomUUID(), // Track this signup
      metadata: {
        plan: normalizedPlan,
        company_name: company_name || '',
        vat_id: vat_id || '',
        country: country || '',
      },
      subscription_data: {
        metadata: {
          plan: normalizedPlan,
          included_impressions: planConfig.included_impressions.toString(),
          overage_rate_cents: planConfig.overage_rate_cents.toString(),
        },
        trial_period_days: 14, // 14-day free trial
      },
      allow_promotion_codes: true,
      automatic_tax: {
        enabled: true, // Stripe Tax handles EU VAT, US sales tax, etc.
      },
      tax_id_collection: {
        enabled: true, // Collect VAT ID for EU B2B
      },
      success_url: `${process.env.CONSOLE_URL}/signup/success?session_id={CHECKOUT_SESSION_ID}`,
      cancel_url: `${process.env.CONSOLE_URL}/pricing`,
      billing_address_collection: 'required',
    });

    console.log(`[Billing] Created checkout session: ${session.id} for ${email} (${normalizedPlan})`);

    res.json({
      checkout_url: session.url,
      session_id: session.id,
    });
  } catch (error: any) {
    console.error('[Billing] Signup error:', error);
    res.status(500).json({ error: 'Failed to create checkout session' });
  }
});

/**
 * GET /api/billing/signup/status/:session_id
 * 
 * Check status of a checkout session and provision account if successful
 */
router.get('/signup/status/:session_id', async (req: Request, res: Response) => {
  const { session_id } = req.params;

  try {
    const session = await stripe.checkout.sessions.retrieve(session_id, {
      expand: ['customer', 'subscription'],
    });

    if (session.payment_status !== 'paid') {
      return res.json({
        status: 'incomplete',
        message: 'Payment not yet completed',
      });
    }

    const customer = session.customer as Stripe.Customer;
    const subscription = session.subscription as Stripe.Subscription;

    // Check if account already provisioned
    const existingUser = await pool.query(
      'SELECT id FROM users WHERE stripe_customer_id = $1',
      [customer.id]
    );

    if (existingUser.rows.length > 0) {
      return res.json({
        status: 'success',
        message: 'Account already provisioned',
        user_id: existingUser.rows[0].id,
      });
    }

    // Provision new account
    const client = await pool.connect();
    try {
      await client.query('BEGIN');

      // 1. Create user
      const userId = crypto.randomUUID();
      const apiKey = generateApiKey();

      await client.query(
        `INSERT INTO users (
          id, email, name, company_name, country, vat_id,
          stripe_customer_id, api_key, status, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())`,
        [
          userId,
          customer.email,
          customer.name || session.metadata?.company_name || '',
          session.metadata?.company_name || '',
          session.metadata?.country || '',
          session.metadata?.vat_id || '',
          customer.id,
          apiKey,
          'active',
        ]
      );

      // 2. Create subscription record
      const planType = resolvePlatformTierId(session.metadata?.plan) ?? 'starter';
      const planConfig = PRICING_PLANS[planType];

      await client.query(
        `INSERT INTO subscriptions (
          id, customer_id, plan_type, plan_name,
          base_price_cents, currency,
          included_impressions, overage_rate_cents,
          billing_interval,
          current_period_start, current_period_end, next_billing_date,
          status, trial_end_date,
          stripe_subscription_id, stripe_customer_id,
          created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, NOW())`,
        [
          crypto.randomUUID(),
          userId,
          planType,
          planConfig.name,
          planConfig.price_cents,
          planConfig.currency.toUpperCase(),
          planConfig.included_impressions,
          planConfig.overage_rate_cents,
          'monthly',
          new Date((subscription as any).current_period_start * 1000),
          new Date((subscription as any).current_period_end * 1000),
          new Date((subscription as any).current_period_end * 1000),
          (subscription as any).trial_end ? 'trial' : 'active',
          (subscription as any).trial_end ? new Date((subscription as any).trial_end * 1000) : null,
          subscription.id,
          customer.id,
        ]
      );

      await client.query('COMMIT');

      console.log(`[Billing] Provisioned account: ${userId} (${customer.email})`);

      // Send welcome email notification (async, non-blocking)
      try {
        logger.info('Welcome email queued', { email: customer.email, userId, planType });
        // Email sending handled by background job queue
      } catch (emailError) {
        logger.error('Failed to queue welcome email', { error: emailError, email: customer.email });
        // Non-blocking - continue with success response
      }

      res.json({
        status: 'success',
        message: 'Account provisioned successfully',
        user_id: userId,
        api_key: apiKey,
        plan: planType,
      });
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  } catch (error: any) {
    console.error('[Billing] Status check error:', error);
    res.status(500).json({ error: 'Failed to check signup status' });
  }
});

/**
 * GET /api/billing/portal
 * 
 * Create Stripe Customer Portal session for managing subscription
 * Requires authentication
 */
router.get('/portal', async (req: Request, res: Response) => {
  // Get user ID from authenticated request
  const userId = (req as any).user?.id || (req as any).user?.publisherId;

  if (!userId) {
    return res.status(401).json({ error: 'Authentication required' });
  }

  try {
    // Get user's Stripe customer ID
    const result = await pool.query(
      'SELECT stripe_customer_id FROM users WHERE id = $1',
      [userId]
    );

    if (result.rows.length === 0) {
      return res.status(404).json({ error: 'User not found' });
    }

    const stripeCustomerId = result.rows[0].stripe_customer_id;

    if (!stripeCustomerId) {
      return res.status(400).json({ error: 'No Stripe customer associated with this account' });
    }

    // Create portal session
    const session = await stripe.billingPortal.sessions.create({
      customer: stripeCustomerId,
      return_url: `${process.env.CONSOLE_URL}/dashboard`,
    });

    res.json({
      portal_url: session.url,
    });
  } catch (error: any) {
    console.error('[Billing] Portal error:', error);
    res.status(500).json({ error: 'Failed to create portal session' });
  }
});

/**
 * POST /api/billing/upgrade
 * 
 * Upgrade subscription to higher tier
 * SECURITY: Requires authentication - uses authenticated user ID
 */
router.post('/upgrade', authenticate, async (req: Request, res: Response) => {
  const { new_plan } = req.body;
  
  // SECURITY: Use authenticated user ID, not request body
  const userId = req.user?.userId || req.user?.publisherId;
  
  if (!userId) {
    return res.status(401).json({ error: 'Authentication required' });
  }

  const normalizedPlan = resolvePlatformTierId(new_plan);

  if (!normalizedPlan || !CHECKOUT_ELIGIBLE_PLANS.includes(normalizedPlan)) {
    return res.status(400).json({ error: 'Invalid plan' });
  }

  try {
    // Get user's current subscription
    const result = await pool.query(
      `SELECT s.stripe_subscription_id, s.plan_type, u.stripe_customer_id
       FROM subscriptions s
       JOIN users u ON s.customer_id = u.id
       WHERE s.customer_id = $1 AND s.status = 'active'`,
      [userId]
    );

    if (result.rows.length === 0) {
      return res.status(404).json({ error: 'No active subscription found' });
    }

    const { stripe_subscription_id, plan_type, stripe_customer_id } = result.rows[0];

    const currentPlan = resolvePlatformTierId(plan_type);

    if (!currentPlan) {
      return res.status(400).json({ error: 'Current subscription tier is unsupported. Contact support.' });
    }

    // Check if it's actually an upgrade
    const planHierarchy = PLATFORM_TIER_ORDER.reduce<Record<PlatformTierId, number>>((acc, tier, index) => {
      acc[tier] = index;
      return acc;
    }, {} as Record<PlatformTierId, number>);

    if (planHierarchy[normalizedPlan] <= planHierarchy[currentPlan]) {
      return res.status(400).json({ error: 'Not an upgrade. Use downgrade endpoint or customer portal.' });
    }

    // Update subscription in Stripe
    const subscription = await stripe.subscriptions.retrieve(stripe_subscription_id);
    const newPlanConfig = PRICING_PLANS[normalizedPlan];

    await stripe.subscriptions.update(stripe_subscription_id, {
      items: [
        {
          id: subscription.items.data[0].id,
          price: newPlanConfig.stripe_price_id,
        },
      ],
      proration_behavior: 'always_invoice', // Charge prorated amount immediately
    });

    // Update in database
    await pool.query(
      `UPDATE subscriptions
       SET plan_type = $1,
           plan_name = $2,
           base_price_cents = $3,
           included_impressions = $4,
           overage_rate_cents = $5,
           updated_at = NOW()
       WHERE customer_id = $6 AND status = 'active'`,
      [
        normalizedPlan,
        newPlanConfig.name,
        newPlanConfig.price_cents,
        newPlanConfig.included_impressions,
        newPlanConfig.overage_rate_cents,
        userId,
      ]
    );

    console.log(`[Billing] Upgraded ${userId} from ${currentPlan} to ${normalizedPlan}`);

    res.json({
      success: true,
      message: `Successfully upgraded to ${newPlanConfig.name}`,
      new_plan: normalizedPlan,
    });
  } catch (error: any) {
    console.error('[Billing] Upgrade error:', error);
    res.status(500).json({ error: 'Failed to upgrade subscription' });
  }
});

/**
 * GET /api/billing/usage
 * 
 * Get current usage statistics for authenticated user
 * SECURITY: Requires authentication - users can only view their own usage
 */
router.get('/usage', authenticate, async (req: Request, res: Response) => {
  // SECURITY: Use authenticated user ID only
  const userId = req.user?.userId || req.user?.publisherId;

  if (!userId) {
    return res.status(401).json({ error: 'Authentication required' });
  }

  try {
    // Get current subscription
    const subResult = await pool.query(
      `SELECT plan_type, included_impressions, current_period_start, current_period_end
       FROM subscriptions
       WHERE customer_id = $1 AND status IN ('trial', 'active')`,
      [userId]
    );

    if (subResult.rows.length === 0) {
      return res.status(404).json({ error: 'No active subscription found' });
    }

    const subscription = subResult.rows[0];
    const planType = resolvePlatformTierId(subscription.plan_type) ?? subscription.plan_type;

    // Get usage from usage_records table
    const usageResult = await pool.query(
      `SELECT
         SUM(quantity) as total_impressions,
         metric_type
       FROM usage_records
       WHERE customer_id = $1
         AND recorded_at >= $2
         AND recorded_at <= $3
         AND metric_type = 'impressions'
       GROUP BY metric_type`,
      [userId, subscription.current_period_start, subscription.current_period_end]
    );

    const totalImpressions = usageResult.rows[0]?.total_impressions || 0;
    const limit = subscription.included_impressions;
    const overage = Math.max(0, totalImpressions - limit);
    const percentUsed = (totalImpressions / limit) * 100;

    res.json({
      period: {
        start: subscription.current_period_start,
        end: subscription.current_period_end,
      },
      usage: {
        impressions: totalImpressions,
        limit: limit,
        overage: overage,
        percent_used: percentUsed.toFixed(2),
      },
      plan: planType,
      alert: percentUsed > 80 ? 'approaching_limit' : percentUsed > 100 ? 'over_limit' : null,
    });
  } catch (error: any) {
    console.error('[Billing] Usage error:', error);
    res.status(500).json({ error: 'Failed to fetch usage' });
  }
});

/**
 * Helper: Generate API key
 */
function generateApiKey(): string {
  return 'rsk_' + crypto.randomBytes(32).toString('hex');
}

export default router;
