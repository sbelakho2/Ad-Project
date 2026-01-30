import express, { Request, Response } from 'express';
import Stripe from 'stripe';
import { Pool } from 'pg';
import { PaymentReconciliationService } from '../services/accounting/PaymentReconciliationService';

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

/**
 * Stripe Webhook Endpoint
 * 
 * Handles all Stripe webhook events for payment reconciliation.
 * 
 * Setup:
 * 1. Go to Stripe Dashboard → Developers → Webhooks
 * 2. Add endpoint: https://yourdomain.com/api/webhooks/stripe
 * 3. Select events:
 *    - invoice.paid
 *    - invoice.payment_failed
 *    - payment_intent.succeeded
 *    - charge.succeeded
 *    - charge.refunded
 * 4. Copy webhook secret to .env as STRIPE_WEBHOOK_SECRET
 */
router.post('/stripe', async (req: Request, res: Response) => {
  const signature = req.headers['stripe-signature'];

  if (!signature) {
    console.error('[Webhook] Missing Stripe signature header');
    return res.status(400).send('Missing signature');
  }

  // SECURITY: Require webhook secret in production to prevent signature bypass
  const webhookSecret = process.env.STRIPE_WEBHOOK_SECRET;
  if (!webhookSecret) {
    console.error('[Webhook] STRIPE_WEBHOOK_SECRET is not configured');
    return res.status(500).send('Webhook secret not configured');
  }

  let event: Stripe.Event;

  try {
    // Verify webhook signature
    event = stripe.webhooks.constructEvent(
      req.body,
      signature,
      webhookSecret
    );
  } catch (error: any) {
    console.error('[Webhook] Signature verification failed:', error.message);
    return res.status(400).send(`Webhook Error: ${error.message}`);
  }

  console.log(`[Webhook] Received event: ${event.type} (${event.id})`);

  try {
    // Initialize payment reconciliation service
    const reconciliationService = new PaymentReconciliationService(pool, stripe);

    // Process the webhook
    await reconciliationService.handleStripeWebhook(event);

    console.log(`[Webhook] Successfully processed: ${event.type}`);
    res.json({ received: true, eventId: event.id });
  } catch (error: any) {
    console.error(`[Webhook] Processing error:`, error);
    
    // Return 200 to acknowledge receipt even on error
    // (prevents Stripe from retrying immediately)
    res.status(200).json({ 
      received: true, 
      eventId: event.id, 
      error: error.message 
    });
  }
});

/**
 * Webhook Test Endpoint (Development Only)
 * 
 * Simulates a Stripe webhook for testing payment reconciliation.
 * DO NOT expose in production!
 */
if (process.env.NODE_ENV === 'development') {
  router.post('/stripe/test', async (req: Request, res: Response) => {
    console.log('[Webhook Test] Received test webhook');

    try {
      const reconciliationService = new PaymentReconciliationService(pool, stripe);

      // Create a fake Stripe event
      const testEvent: Stripe.Event = {
        id: `evt_test_${Date.now()}`,
        object: 'event',
        api_version: '2025-10-29.clover',
        created: Math.floor(Date.now() / 1000),
        type: req.body.type || 'invoice.paid',
        data: {
          object: req.body.data || {},
        },
        livemode: false,
        pending_webhooks: 0,
        request: {
          id: null,
          idempotency_key: null,
        },
      };

      await reconciliationService.handleStripeWebhook(testEvent);

      res.json({ success: true, event: testEvent });
    } catch (error: any) {
      console.error('[Webhook Test] Error:', error);
      res.status(500).json({ error: error.message });
    }
  });
}

export default router;
