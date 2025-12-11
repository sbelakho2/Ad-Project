// Types are augmented via ../types/express.d.ts; no triple-slash needed
import { Request, Response, NextFunction } from 'express';
import Stripe from 'stripe';
import { AppError } from '../middleware/errorHandler';
import { logger } from '../utils/logger';
import { usageMeteringService } from '../services/billing/UsageMeteringService';
import { invoiceService } from '../services/invoiceService';
import { reconciliationService } from '../services/reconciliationService';
import { migrationAssistantService } from '../services/billing/migrationAssistantService';
import { platformFeeService } from '../services/billing/platformFeeService';
import { getBillingPolicySnapshot } from '../config/billingPolicy';
import { query } from '../utils/postgres';

let stripeClient: Stripe | null = null;

const getStripeClient = (): Stripe => {
  if (stripeClient) return stripeClient;

  const secretKey = process.env.STRIPE_SECRET_KEY;
  if (!secretKey) {
    throw new AppError('Stripe secret key not configured', 500);
  }

  stripeClient = new Stripe(secretKey, {
    apiVersion: '2025-10-29.clover',
  });

  return stripeClient;
};

/**
 * GET /api/v1/billing/usage/current
 * Returns current billing period usage with overage calculations
 */
export const getCurrentUsage = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const customerId = req.user?.userId;
    
    if (!customerId) {
      throw new AppError('User not authenticated', 401);
    }

    // Get usage metrics
    const usage = await usageMeteringService.getCurrentPeriodUsage(customerId);
    
    // Get overage calculations
    const overages = await usageMeteringService.calculateOverages(customerId);
    
    // Get subscription details for plan limits
    const subscription = await usageMeteringService.getSubscriptionDetails(customerId);

    res.json({
      success: true,
      data: {
        period: {
          start: usage.period_start,
          end: usage.period_end,
        },
        plan: subscription.plan_type,
        usage: {
          impressions: usage.impressions,
          api_calls: usage.api_calls,
          data_transfer_gb: usage.data_transfer_gb,
        },
        limits: {
          impressions: subscription.included_impressions,
          api_calls: subscription.included_api_calls || 0,
          data_transfer_gb: subscription.included_data_transfer_gb || 0,
        },
        overages: {
          impressions: overages.impressions_overage,
          impressions_cost_cents: overages.impressions_overage_cost_cents,
          api_calls: overages.api_calls_overage,
          api_calls_cost_cents: overages.api_calls_overage_cost_cents,
          data_transfer_gb: overages.data_transfer_overage_gb,
          data_transfer_cost_cents: overages.data_transfer_overage_cost_cents,
          total_cost_cents: overages.total_overage_cost_cents,
        },
      },
    });
  } catch (error) {
    logger.error('Error fetching current usage', { error, userId: req.user?.userId });
    next(error);
  }
};

/**
 * GET /api/v1/billing/invoices
 * List invoices with pagination and filtering
 */
export const listInvoices = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const customerId = req.user?.userId;
    
    if (!customerId) {
      throw new AppError('User not authenticated', 401);
    }

    const { status, from, to, page = '1', limit = '20' } = req.query;
    
    const pageNum = parseInt(page as string, 10);
    const limitNum = parseInt(limit as string, 10);
    
    if (isNaN(pageNum) || pageNum < 1) {
      throw new AppError('Invalid page number', 400);
    }
    
    if (isNaN(limitNum) || limitNum < 1 || limitNum > 100) {
      throw new AppError('Invalid limit (must be 1-100)', 400);
    }

    const filters = {
      status: status as string | undefined,
      from: from as string | undefined,
      to: to as string | undefined,
    };

    const result = await invoiceService.listInvoices(customerId, pageNum, limitNum, filters);

    res.json({
      success: true,
      data: result.invoices,
      pagination: {
        page: pageNum,
        limit: limitNum,
        total: result.total,
        pages: Math.ceil(result.total / limitNum),
      },
    });
  } catch (error) {
    logger.error('Error listing invoices', { error, userId: req.user?.userId });
    next(error);
  }
};

/**
 * GET /api/v1/billing/invoices/:id
 * Get specific invoice details
 */
export const getInvoice = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const customerId = req.user?.userId;
    const { id } = req.params;
    
    if (!customerId) {
      throw new AppError('User not authenticated', 401);
    }

    const invoice = await invoiceService.getInvoice(id, customerId);
    
    if (!invoice) {
      throw new AppError('Invoice not found', 404);
    }

    res.json({
      success: true,
      data: invoice,
    });
  } catch (error) {
    logger.error('Error fetching invoice', { error, userId: req.user?.userId, invoiceId: req.params.id });
    next(error);
  }
};

/**
 * GET /api/v1/billing/invoices/:id/pdf
 * Stream invoice PDF
 */
export const getInvoicePDF = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const customerId = req.user?.userId;
    const { id } = req.params;
    
    if (!customerId) {
      throw new AppError('User not authenticated', 401);
    }

    const invoice = await invoiceService.getInvoice(id, customerId);
    
    if (!invoice) {
      throw new AppError('Invoice not found', 404);
    }

    // Generate ETag from invoice data
    const etag = invoiceService.generateInvoiceETag(invoice);
    
    // Check if client has cached version
    if (req.headers['if-none-match'] === etag) {
      res.status(304).end();
      return;
    }

    // Generate PDF
    const pdfBuffer = await invoiceService.generateInvoicePDF(invoice);

    res.setHeader('Content-Type', 'application/pdf');
    res.setHeader('Content-Disposition', `attachment; filename="invoice-${invoice.invoice_number}.pdf"`);
    res.setHeader('ETag', etag);
    res.setHeader('Cache-Control', 'private, max-age=3600');
    res.send(pdfBuffer);
  } catch (error) {
    logger.error('Error generating invoice PDF', { error, userId: req.user?.userId, invoiceId: req.params.id });
    next(error);
  }
};

/**
 * POST /api/v1/billing/migration/request
 * Capture billing migration context for ops to review
 */
export const requestMigration = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const customerId = req.user?.userId;

    if (!customerId) {
      throw new AppError('User not authenticated', 401);
    }

    const { channel, notes } = req.body || {};

    if (typeof notes !== 'string' || notes.trim().length < 20) {
      throw new AppError('Provide migration notes with at least 20 characters of context.', 400);
    }

    const targetChannel = channel === 'production' ? 'production' : 'sandbox';

    const request = await migrationAssistantService.createRequest({
      customerId,
      accountEmail: req.user?.email ?? null,
      channel: targetChannel,
      notes: notes.trim(),
    });

    res.status(202).json({
      success: true,
      data: request,
    });
  } catch (error) {
    logger.error('Error handling billing migration request', {
      error,
      userId: req.user?.userId,
    });
    next(error);
  }
};

/**
 * POST /api/v1/billing/portal
 * Creates a Stripe Customer Portal session for the authenticated user.
 */
export const createPortalSession = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const userId = req.user?.userId;
    const email = req.user?.email || undefined;

    if (!userId) {
      throw new AppError('User not authenticated', 401);
    }

    const origin =
      (req.get('origin') && req.get('origin')!.trim()) ||
      (process.env.CONSOLE_URL && process.env.CONSOLE_URL.trim()) ||
      (process.env.WEBSITE_URL && process.env.WEBSITE_URL.trim()) ||
      '';

    if (!origin) {
      throw new AppError('Portal return URL not configured', 500);
    }

    const returnUrl = `${origin.replace(/\/+$/, '')}/billing/settings`;
    const stripe = getStripeClient();

    const result = await query<{ stripe_customer_id: string | null }>(
      'SELECT stripe_customer_id FROM users WHERE id = $1',
      [userId]
    );

    if (result.rows.length === 0) {
      throw new AppError('User not found', 404);
    }

    let stripeCustomerId = result.rows[0].stripe_customer_id;

    if (!stripeCustomerId) {
      const customer = await stripe.customers.create({
        email,
        metadata: {
          user_id: userId,
        },
      });
      stripeCustomerId = customer.id;

      await query('UPDATE users SET stripe_customer_id = $2 WHERE id = $1', [userId, stripeCustomerId]);
    }

    const session = await stripe.billingPortal.sessions.create({
      customer: stripeCustomerId,
      return_url: returnUrl,
    });

    res.json({
      success: true,
      url: session.url,
    });
  } catch (error) {
    logger.error('Error creating Stripe portal session', {
      error,
      userId: req.user?.userId,
    });
    next(error);
  }
};

/**
 * POST /api/v1/billing/reconcile
 * Trigger billing reconciliation (admin only)
 */
export const reconcileBilling = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    // Enforce header-based idempotency per FIX-11 (648)
    const idempotencyKeyHeader = req.header('Idempotency-Key') || req.header('idempotency-key');
    const idempotencyKey = idempotencyKeyHeader?.trim();

    if (!idempotencyKey) {
      throw new AppError('Idempotency-Key header required', 400);
    }

    // Check if already processed
    const existing = await reconciliationService.checkIdempotencyKey(idempotencyKey);
    if (existing) {
      res.json({
        success: true,
        data: existing,
        message: 'Reconciliation already processed',
      });
      return;
    }

    // Run reconciliation
    const result = await reconciliationService.reconcile();
    
    // Store idempotency record
    await reconciliationService.storeIdempotencyKey(idempotencyKey, result);
    
    // Emit audit event
    logger.info('Billing reconciliation triggered', {
      actor: req.user?.email,
      result,
      idempotencyKey,
    });

    res.json({
      success: true,
      data: result,
    });
  } catch (error) {
    logger.error('Error during billing reconciliation', { error, user: req.user });
    next(error);
  }
};

/**
 * POST /api/v1/billing/platform-fees/calculate
 * Calculate platform fee for BYO tiers
 */
export const calculatePlatformFee = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const { gross_revenue_cents, custom_enterprise_rate_percent } = req.body || {};

    if (typeof gross_revenue_cents !== 'number' || gross_revenue_cents < 0) {
      throw new AppError('Invalid gross_revenue_cents (must be non-negative number)', 400);
    }

    const calculation = platformFeeService.calculatePlatformFee(gross_revenue_cents, {
      customEnterpriseRatePercent:
        typeof custom_enterprise_rate_percent === 'number'
          ? custom_enterprise_rate_percent
          : undefined,
    });

    res.json({
      success: true,
      data: {
        ...calculation,
        tier: {
          id: calculation.tier.id,
          name: calculation.tier.name,
          description: calculation.tier.description,
          min_revenue_cents: calculation.tier.min_revenue_cents,
          max_revenue_cents: calculation.tier.max_revenue_cents,
          rate: calculation.tier.rate,
          min_rate: calculation.tier.min_rate,
          max_rate: calculation.tier.max_rate,
          example_fee_note: calculation.tier.example_fee_note,
        },
      },
    });
  } catch (error) {
    logger.error('Error calculating platform fee', { error, body: req.body });
    next(error);
  }
};

/**
 * GET /api/v1/billing/platform-fees/tiers
 * Get platform fee tier configuration
 */
export const getPlatformTiers = async (
  _req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const tiers = platformFeeService.getTiers();

    res.json({
      success: true,
      data: tiers.map((tier) => ({
        id: tier.id,
        name: tier.name,
        description: tier.description,
        min_revenue_cents: tier.min_revenue_cents,
        max_revenue_cents: tier.max_revenue_cents,
        rate: tier.rate,
        min_rate: tier.min_rate,
        max_rate: tier.max_rate,
        example_fee_note: tier.example_fee_note,
      })),
      metadata: {
        currency: 'USD',
        interpretation: 'Rates apply per app per month based on mediated revenue',
      },
    });
  } catch (error) {
    logger.error('Error fetching platform tier configs', { error });
    next(error);
  }
};

/**
 * GET /api/v1/billing/policy
 * Returns the Stripe-first billing policy snapshot used across surfaces
 */
export const getBillingPolicy = async (
  _req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const policy = getBillingPolicySnapshot();

    res.json({
      success: true,
      data: policy,
    });
  } catch (error) {
    logger.error('Error building billing policy payload', { error });
    next(error);
  }
};
