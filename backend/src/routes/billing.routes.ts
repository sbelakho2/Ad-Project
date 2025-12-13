import { Router } from 'express';
import { authenticate, authorize } from '../middleware/auth';
import { requireFeature } from '../utils/featureFlags';
import * as billingController from '../controllers/billing.controller';
import { authRateLimiter } from '../middleware/redisRateLimiter';

const router = Router();

// All billing endpoints require billing feature to be enabled
router.use(requireFeature('billingEnabled'));

/**
 * GET /api/v1/billing/policy
 * Public snapshot describing the Stripe-first billing policy
 */
router.get('/policy', billingController.getBillingPolicy);

// All remaining billing endpoints require authentication
router.use(authenticate);

/**
 * POST /api/v1/billing/portal
 * Create Stripe Customer Portal session
 */
router.post('/portal', authRateLimiter, billingController.createPortalSession);

/**
 * GET /api/v1/billing/usage/current
 * Get current billing period usage for authenticated user
 */
router.get('/usage/current', billingController.getCurrentUsage);

/**
 * GET /api/v1/billing/invoices
 * List invoices for authenticated user with pagination and filters
 */
// List invoices can be expensive: apply a modest rate limit
router.get('/invoices', authRateLimiter, billingController.listInvoices);

/**
 * GET /api/v1/billing/invoices/:id
 * Get specific invoice details
 */
router.get('/invoices/:id', billingController.getInvoice);

/**
 * GET /api/v1/billing/invoices/:id/pdf
 * Download invoice as PDF
 */
// PDF generation is relatively heavy; protect with rate limit
router.get('/invoices/:id/pdf', authRateLimiter, billingController.getInvoicePDF);

/**
 * POST /api/v1/billing/migration/request
 * Capture migration assistant requests
 */
router.post('/migration/request', billingController.requestMigration);

/**
 * POST /api/v1/billing/reconcile
 * DEPRECATED: Billing reconciliation is now operator-only at /api/v1/admin/billing/reconcile
 * Customer admins cannot trigger system-wide reconciliation
 */
router.post(
  '/reconcile',
  authorize(['admin']),
  authRateLimiter,
  (_req, res) => {
    res.status(403).json({
      success: false,
      error: 'Billing reconciliation is an operator-only function. Contact support if you need assistance.',
    });
  }
);

/**
 * Platform fee tier endpoints (BYO pricing)
 */
router.post('/platform-fees/calculate', billingController.calculatePlatformFee);
router.get('/platform-fees/tiers', billingController.getPlatformTiers);

// Legacy endpoints kept temporarily for backwards compatibility
router.post('/revenue-share/calculate', billingController.calculatePlatformFee);
router.get('/tiers', billingController.getPlatformTiers);

export default router;
