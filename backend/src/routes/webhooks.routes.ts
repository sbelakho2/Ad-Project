import { Router, Request, Response } from 'express';
import Stripe from 'stripe';
import { logger } from '../utils/logger';
import { invoiceService } from '../services/invoiceService';
import { Pool, PoolClient } from 'pg';
import { dunningManagementService } from '../../services/billing/DunningManagementService';
// REVOLUTIONARY: Import sales automation webhook handler
import { handleResendWebhook, processReply } from '../../services/sales-automation/webhook';

const router = Router();

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

const db = new Pool({
  connectionString: process.env.DATABASE_URL,
});

/**
 * Execute a handler within a database transaction
 */
async function withTransaction<T>(
  handler: (client: PoolClient) => Promise<T>
): Promise<T> {
  const client = await db.connect();
  try {
    await client.query('BEGIN');
    const result = await handler(client);
    await client.query('COMMIT');
    return result;
  } catch (error) {
    await client.query('ROLLBACK');
    throw error;
  } finally {
    client.release();
  }
}

// Webhook secret is required for signature verification - fail fast if missing
const webhookSecret = process.env.STRIPE_WEBHOOK_SECRET;
if (!webhookSecret && process.env.NODE_ENV === 'production') {
  logger.error('STRIPE_WEBHOOK_SECRET is required for webhook signature verification');
}

/**
 * POST /api/v1/webhooks/stripe
 * Handle Stripe webhook events
 * 
 * Note: This endpoint must use raw body, not JSON parsed body
 */
router.post('/stripe', async (req: Request, res: Response) => {
  // Validate webhook secret is configured
  if (!webhookSecret) {
    logger.error('Stripe webhook: STRIPE_WEBHOOK_SECRET not configured');
    return res.status(500).send('Server configuration error');
  }

  const sig = req.headers['stripe-signature'];

  if (!sig) {
    logger.warn('Stripe webhook: missing signature');
    return res.status(400).send('Missing signature');
  }

  let event: Stripe.Event;

  try {
    const stripe = getStripeClient();
    // Verify webhook signature
    event = stripe.webhooks.constructEvent(
      req.body,
      sig as string,
      webhookSecret
    );
  } catch (err) {
    logger.error('Stripe webhook signature verification failed', {
      error: err instanceof Error ? err.message : 'Unknown error',
    });
    return res.status(400).send(`Webhook Error: ${err instanceof Error ? err.message : 'Unknown error'}`);
  }

  // Check for duplicate event (idempotency)
  const existingEvent = await db.query(
    `SELECT id FROM stripe_webhook_events WHERE stripe_event_id = $1`,
    [event.id]
  );

  if (existingEvent.rows.length > 0) {
    logger.info('Stripe webhook: duplicate event ignored', { eventId: event.id });
    return res.json({ received: true, duplicate: true });
  }

  // Store event for idempotency
  await db.query(
    `INSERT INTO stripe_webhook_events (stripe_event_id, event_type, processed, created_at)
     VALUES ($1, $2, false, NOW())`,
    [event.id, event.type]
  );

  // Handle the event
  try {
    switch (event.type) {
      case 'invoice.created':
        await handleInvoiceCreated(event.data.object as Stripe.Invoice);
        break;
      
      case 'invoice.finalized':
        await handleInvoiceFinalized(event.data.object as Stripe.Invoice);
        break;
      
      case 'invoice.paid':
        await handleInvoicePaid(event.data.object as Stripe.Invoice);
        break;
      
      case 'invoice.payment_succeeded':
        await handleInvoicePaymentSucceeded(event.data.object as Stripe.Invoice);
        break;
      
      case 'invoice.payment_failed':
        await handleInvoicePaymentFailed(event.data.object as Stripe.Invoice);
        break;
      
      case 'charge.refunded':
        await handleChargeRefunded(event.data.object as Stripe.Charge);
        break;
      
      case 'payment_method.attached':
        await handlePaymentMethodAttached(event.data.object as Stripe.PaymentMethod);
        break;
      
      case 'customer.subscription.created':
        await handleSubscriptionCreated(event.data.object as Stripe.Subscription);
        break;
      
      case 'customer.subscription.updated':
        await handleSubscriptionUpdated(event.data.object as Stripe.Subscription);
        break;
      
      case 'customer.subscription.deleted':
        await handleSubscriptionDeleted(event.data.object as Stripe.Subscription);
        break;
      
      default:
        logger.info('Stripe webhook: unhandled event type', { type: event.type });
    }

    // Mark event as processed
    await db.query(
      `UPDATE stripe_webhook_events SET processed = true, processed_at = NOW()
       WHERE stripe_event_id = $1`,
      [event.id]
    );

    logger.info('Stripe webhook processed successfully', {
      eventId: event.id,
      type: event.type,
    });
  } catch (error) {
    logger.error('Error processing Stripe webhook', {
      eventId: event.id,
      type: event.type,
      error: error instanceof Error ? error.message : 'Unknown error',
    });

    // Mark as failed for retry
    await db.query(
      `UPDATE stripe_webhook_events 
       SET processed = false, 
           error_message = $1,
           retry_count = COALESCE(retry_count, 0) + 1,
           last_error_at = NOW()
       WHERE stripe_event_id = $2`,
      [error instanceof Error ? error.message : 'Unknown error', event.id]
    );

    // Return 500 so Stripe will retry
    return res.status(500).json({ error: 'Processing failed, will retry' });
  }

  res.json({ received: true });
});

/**
 * Handle invoice.created event
 */
async function handleInvoiceCreated(invoice: Stripe.Invoice): Promise<void> {
  logger.info('Handling invoice.created', { invoiceId: invoice.id });
  
  await withTransaction(async (client) => {
    // Sync invoice to database
    await invoiceService.syncInvoiceFromStripe(invoice.id);
    
    // Log to audit trail
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['invoice_created', JSON.stringify({ stripe_invoice_id: invoice.id })]
    );
  });
}

/**
 * Handle invoice.finalized event
 */
async function handleInvoiceFinalized(invoice: Stripe.Invoice): Promise<void> {
  logger.info('Handling invoice.finalized', { invoiceId: invoice.id });
  
  await withTransaction(async (client) => {
    await invoiceService.syncInvoiceFromStripe(invoice.id);
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['invoice_finalized', JSON.stringify({ stripe_invoice_id: invoice.id })]
    );
  });
}

/**
 * Handle invoice.paid event (distinct from payment_succeeded for Stripe billing)
 */
async function handleInvoicePaid(invoice: Stripe.Invoice): Promise<void> {
  logger.info('Handling invoice.paid', { invoiceId: invoice.id });
  
  await withTransaction(async (client) => {
    // Update invoice status to paid
    await client.query(
      `UPDATE invoices 
       SET status = 'paid', paid_at = NOW(), updated_at = NOW()
       WHERE stripe_invoice_id = $1`,
      [invoice.id]
    );
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['invoice_paid', JSON.stringify({ stripe_invoice_id: invoice.id, amount: invoice.total })]
    );
  });
}

/**
 * Handle invoice.payment_succeeded event
 */
async function handleInvoicePaymentSucceeded(invoice: Stripe.Invoice): Promise<void> {
  logger.info('Handling invoice.payment_succeeded', { invoiceId: invoice.id });
  
  await withTransaction(async (client) => {
    // Update invoice status to paid
    await client.query(
      `UPDATE invoices 
       SET status = 'paid', paid_at = NOW(), updated_at = NOW()
       WHERE stripe_invoice_id = $1`,
      [invoice.id]
    );
    
    // Reset failed payment counter and reactivate subscription
    const subscriptionId = (invoice as any).subscription;
    if (subscriptionId) {
      await client.query(
        `UPDATE subscriptions 
         SET failed_payment_count = 0, status = 'active'
         WHERE stripe_subscription_id = $1`,
        [subscriptionId]
      );
      
      // Clear any active dunning for this subscription
      await client.query(
        `UPDATE dunning_attempts 
         SET status = 'succeeded'
         WHERE subscription_id = $1 AND status IN ('pending', 'retrying')`,
        [subscriptionId]
      );
    }
    
    // Emit success event for email
    await client.query(
      `INSERT INTO events (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      [
        'invoice.payment_succeeded',
        JSON.stringify({
          stripe_invoice_id: invoice.id,
          customer_id: invoice.customer,
          amount: invoice.total,
        }),
      ]
    );
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['payment_succeeded', JSON.stringify({ stripe_invoice_id: invoice.id, amount: invoice.total })]
    );
  });
}

/**
 * Handle invoice.payment_failed event
 */
async function handleInvoicePaymentFailed(invoice: Stripe.Invoice): Promise<void> {
  logger.warn('Handling invoice.payment_failed', { invoiceId: invoice.id });
  
  await withTransaction(async (client) => {
    const subscriptionId = (invoice as any).subscription;
    
    // Increment failed payment counter
    if (subscriptionId) {
      await client.query(
        `UPDATE subscriptions 
         SET failed_payment_count = failed_payment_count + 1,
             last_failed_payment_at = NOW(),
             status = 'past_due'
         WHERE stripe_subscription_id = $1`,
        [subscriptionId]
      );
    }
    
    // Emit failure event for dunning management
    await client.query(
      `INSERT INTO events (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      [
        'invoice.payment_failed',
        JSON.stringify({
          stripe_invoice_id: invoice.id,
          customer_id: invoice.customer,
          amount: invoice.total,
          attempt_count: invoice.attempt_count,
        }),
      ]
    );
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['payment_failed', JSON.stringify({ stripe_invoice_id: invoice.id })]
    );
  });
  
  // Trigger dunning management (outside transaction to avoid long locks)
  try {
    await dunningManagementService.handlePaymentFailed(invoice);
  } catch (dunningError) {
    logger.error('Failed to trigger dunning management', {
      invoiceId: invoice.id,
      error: dunningError instanceof Error ? dunningError.message : 'Unknown error',
    });
    // Don't re-throw - dunning failure shouldn't fail the webhook
  }
}

/**
 * Handle charge.refunded event
 */
async function handleChargeRefunded(charge: Stripe.Charge): Promise<void> {
  logger.info('Handling charge.refunded', { chargeId: charge.id });
  
  await withTransaction(async (client) => {
    // Update invoice status if linked
    const invoiceId = (charge as any).invoice;
    if (invoiceId) {
      await client.query(
        `UPDATE invoices 
         SET status = 'refunded', updated_at = NOW()
         WHERE stripe_invoice_id = $1`,
        [invoiceId]
      );
    }
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['charge_refunded', JSON.stringify({ charge_id: charge.id, amount: charge.amount_refunded })]
    );
  });
}

/**
 * Handle payment_method.attached event
 */
async function handlePaymentMethodAttached(paymentMethod: Stripe.PaymentMethod): Promise<void> {
  logger.info('Handling payment_method.attached', { 
    paymentMethodId: paymentMethod.id,
    customerId: paymentMethod.customer,
  });
  
  await withTransaction(async (client) => {
    // Record payment method attachment for audit
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      [
        'payment_method_attached',
        JSON.stringify({
          payment_method_id: paymentMethod.id,
          customer_id: paymentMethod.customer,
          type: paymentMethod.type,
          card_last4: paymentMethod.card?.last4,
          card_brand: paymentMethod.card?.brand,
        }),
      ]
    );
    
    // Update customer's default payment method if this is their first
    if (paymentMethod.customer) {
      await client.query(
        `UPDATE users 
         SET default_payment_method_id = COALESCE(default_payment_method_id, $1),
             updated_at = NOW()
         WHERE stripe_customer_id = $2`,
        [paymentMethod.id, paymentMethod.customer]
      );
    }
  });
}

/**
 * Handle customer.subscription.created event
 */
async function handleSubscriptionCreated(subscription: Stripe.Subscription): Promise<void> {
  logger.info('Handling subscription.created', { subscriptionId: subscription.id });
  
  await withTransaction(async (client) => {
    // Check if subscription already exists
    const existing = await client.query(
      `SELECT id FROM subscriptions WHERE stripe_subscription_id = $1`,
      [subscription.id]
    );
    
    if (existing.rows.length === 0) {
      // Create new subscription record
      await client.query(
        `INSERT INTO subscriptions (
           stripe_subscription_id, 
           stripe_customer_id,
           status, 
           current_period_start, 
           current_period_end,
           created_at,
           updated_at
         ) VALUES ($1, $2, $3, $4, $5, NOW(), NOW())`,
        [
          subscription.id,
          subscription.customer,
          subscription.status,
          new Date((subscription as any).current_period_start * 1000),
          new Date((subscription as any).current_period_end * 1000),
        ]
      );
    }
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['subscription_created', JSON.stringify({ stripe_subscription_id: subscription.id })]
    );
  });
}

/**
 * Handle customer.subscription.updated event
 */
async function handleSubscriptionUpdated(subscription: Stripe.Subscription): Promise<void> {
  logger.info('Handling subscription.updated', { subscriptionId: subscription.id });
  
  await withTransaction(async (client) => {
    // Update subscription in database
    await client.query(
      `UPDATE subscriptions
       SET status = $1,
           current_period_start = $2,
           current_period_end = $3,
           updated_at = NOW()
       WHERE stripe_subscription_id = $4`,
      [
        subscription.status,
        new Date((subscription as any).current_period_start * 1000),
        new Date((subscription as any).current_period_end * 1000),
        subscription.id,
      ]
    );
  });
}

/**
 * Handle customer.subscription.deleted event
 */
async function handleSubscriptionDeleted(subscription: Stripe.Subscription): Promise<void> {
  logger.info('Handling subscription.deleted', { subscriptionId: subscription.id });
  
  await withTransaction(async (client) => {
    await client.query(
      `UPDATE subscriptions
       SET status = 'cancelled', 
           cancelled_at = NOW(),
           updated_at = NOW()
       WHERE stripe_subscription_id = $1`,
      [subscription.id]
    );
    
    // Cancel any active dunning attempts
    await client.query(
      `UPDATE dunning_attempts 
       SET status = 'cancelled'
       WHERE subscription_id = $1 AND status IN ('pending', 'retrying')`,
      [subscription.id]
    );
    
    await client.query(
      `INSERT INTO billing_audit (event_type, data, created_at)
       VALUES ($1, $2, NOW())`,
      ['subscription_cancelled', JSON.stringify({ stripe_subscription_id: subscription.id })]
    );
  });
}

// ============================================================================
// REVOLUTIONARY: RESEND WEBHOOK (Sales Automation)
// Handles email opens, clicks, bounces, and spam complaints
// OVERCOMES: "No feedback loop", "Domain tanks without warning"
// ============================================================================

/**
 * POST /api/v1/webhooks/resend
 * Handle Resend email events (opens, clicks, bounces, spam complaints)
 * 
 * Note: This endpoint receives raw JSON from Resend
 * Configure in Resend Dashboard:
 *   URL: https://api.apexmediation.ee/api/v1/webhooks/resend
 *   Events: email.delivered, email.opened, email.clicked, email.bounced, email.complained
 */
router.post('/resend', async (req: Request, res: Response) => {
  try {
    const signature = req.headers['svix-signature'] as string || '';
    const rawBody = Buffer.isBuffer(req.body)
      ? req.body.toString('utf8')
      : typeof req.body === 'string'
        ? req.body
        : JSON.stringify(req.body ?? {});
    
    const result = await handleResendWebhook(rawBody, signature);
    
    if (result.success) {
      logger.info('Resend webhook processed', { message: result.message });
      return res.status(200).json(result);
    } else {
      logger.warn('Resend webhook failed', { message: result.message });
      return res.status(400).json(result);
    }
  } catch (error) {
    logger.error('Resend webhook error', { error: error instanceof Error ? error.message : 'Unknown' });
    return res.status(500).json({ success: false, message: 'Internal error' });
  }
});

/**
 * POST /api/v1/webhooks/resend/reply
 * Handle incoming email replies (via forwarding or Resend inbound)
 * 
 * REVOLUTIONARY: Uses SmartReplyAnalyzer to categorize replies
 */
router.post('/resend/reply', async (req: Request, res: Response) => {
  try {
    const payload = Buffer.isBuffer(req.body)
      ? JSON.parse(req.body.toString('utf8'))
      : typeof req.body === 'string'
        ? JSON.parse(req.body)
        : req.body;

    const { from, subject, body } = payload || {};
    
    if (!from || !body) {
      return res.status(400).json({ success: false, message: 'Missing from or body' });
    }
    
    const result = await processReply(from, subject || '(no subject)', body);
    
    if (result.success) {
      logger.info('Reply processed', { 
        from, 
        category: result.category, 
        sentiment: result.sentiment,
        suggestedAction: result.suggestedAction,
      });
      return res.status(200).json(result);
    } else {
      logger.info('Reply from unknown contact', { from });
      return res.status(200).json({ success: false, message: 'Contact not found' });
    }
  } catch (error) {
    logger.error('Reply processing error', { error: error instanceof Error ? error.message : 'Unknown' });
    return res.status(500).json({ success: false, message: 'Internal error' });
  }
});

export default router;
