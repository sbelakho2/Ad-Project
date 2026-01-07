#!/usr/bin/env ts-node
/**
 * Dunning Retry Cron Job
 * 
 * Runs daily to process failed payment retries for subscriptions in dunning.
 * Implements progressive retry logic with email notifications at each stage.
 * 
 * Schedule: 0 8 * * * (8:00 AM UTC daily - business hours for customer awareness)
 * 
 * This job:
 * 1. Queries dunning_attempts with status 'pending' or 'retrying' where next_retry_at <= NOW()
 * 2. For each, attempts to pay the invoice via Stripe
 * 3. On success: marks dunning as succeeded, reactivates subscription
 * 4. On failure: increments attempt count, schedules next retry or suspends
 */

import Stripe from 'stripe';
import pkg from 'pg';
const { Pool } = pkg;
import { setTimeout } from 'timers/promises';

// Configuration
const STRIPE_SECRET_KEY = process.env.STRIPE_SECRET_KEY || '';
const DATABASE_URL = process.env.DATABASE_URL || 'postgresql://localhost:5432/adtech';

// Dunning configuration
const DUNNING_CONFIG = {
  maxRetries: 3,
  retryScheduleDays: [1, 3, 7], // Days until next retry after each failure
  suspendAfterFailures: true,
  gracePeriodDays: 3,
};

// Validate config
function assertConfig(): void {
  if (!STRIPE_SECRET_KEY) {
    throw new Error('Missing STRIPE_SECRET_KEY environment variable');
  }
}

assertConfig();

const stripe = new Stripe(STRIPE_SECRET_KEY, {
  apiVersion: '2025-10-29.clover' as any,
  typescript: true,
});

const db = new Pool({
  connectionString: DATABASE_URL,
});

interface DunningAttempt {
  id: string;
  customer_id: string;
  subscription_id: string;
  invoice_id: string;
  attempt_number: number;
  next_retry_at: Date;
  status: string;
}

interface DunningResult {
  id: string;
  customerId: string;
  invoiceId: string;
  success: boolean;
  message: string;
  nextAction?: 'retry_scheduled' | 'subscription_suspended' | 'completed';
}

/**
 * Fetch all dunning attempts due for retry
 */
async function fetchDueDunningAttempts(): Promise<DunningAttempt[]> {
  const result = await db.query<DunningAttempt>(
    `SELECT 
       id, customer_id, subscription_id, invoice_id, attempt_number, next_retry_at, status
     FROM dunning_attempts
     WHERE status IN ('pending', 'retrying')
       AND next_retry_at <= NOW()
     ORDER BY next_retry_at ASC
     LIMIT 100`
  );
  return result.rows;
}

/**
 * Retry payment for a single dunning attempt
 */
async function retryPayment(attempt: DunningAttempt): Promise<DunningResult> {
  console.log(`[Dunning] Retrying payment for customer ${attempt.customer_id}, attempt ${attempt.attempt_number + 1}`);
  
  // Mark as retrying
  await db.query(
    `UPDATE dunning_attempts SET status = 'retrying', updated_at = NOW() WHERE id = $1`,
    [attempt.id]
  );
  
  try {
    // Retrieve the invoice from Stripe
    const invoice = await stripe.invoices.retrieve(attempt.invoice_id);
    
    if (invoice.status === 'paid') {
      // Invoice was paid outside of dunning process
      console.log(`[Dunning] Invoice ${invoice.id} is already paid`);
      await markDunningSuccess(attempt);
      return {
        id: attempt.id,
        customerId: attempt.customer_id,
        invoiceId: attempt.invoice_id,
        success: true,
        message: 'Invoice already paid',
        nextAction: 'completed',
      };
    }
    
    if (invoice.status === 'void' || invoice.status === 'uncollectible') {
      // Invoice can't be collected
      console.log(`[Dunning] Invoice ${invoice.id} is ${invoice.status}, cancelling dunning`);
      await db.query(
        `UPDATE dunning_attempts SET status = 'cancelled', updated_at = NOW() WHERE id = $1`,
        [attempt.id]
      );
      return {
        id: attempt.id,
        customerId: attempt.customer_id,
        invoiceId: attempt.invoice_id,
        success: false,
        message: `Invoice is ${invoice.status}`,
        nextAction: 'completed',
      };
    }
    
    // Attempt to pay the invoice
    const paidInvoice = await stripe.invoices.pay(invoice.id, {
      forgive: false,
    });
    
    if (paidInvoice.status === 'paid') {
      console.log(`[Dunning] Payment successful for invoice ${invoice.id}`);
      await markDunningSuccess(attempt);
      await emitSuccessEmail(attempt.customer_id, paidInvoice);
      return {
        id: attempt.id,
        customerId: attempt.customer_id,
        invoiceId: attempt.invoice_id,
        success: true,
        message: 'Payment successful',
        nextAction: 'completed',
      };
    } else {
      // Payment still pending or failed
      console.log(`[Dunning] Payment retry returned status: ${paidInvoice.status}`);
      return await handleRetryFailed(attempt);
    }
  } catch (error: any) {
    console.error(`[Dunning] Error retrying payment:`, error.message);
    return await handleRetryFailed(attempt);
  }
}

/**
 * Handle a failed retry attempt
 */
async function handleRetryFailed(attempt: DunningAttempt): Promise<DunningResult> {
  const nextAttemptNumber = attempt.attempt_number + 1;
  
  if (nextAttemptNumber >= DUNNING_CONFIG.maxRetries) {
    // Max retries reached - suspend subscription
    console.log(`[Dunning] Max retries reached for customer ${attempt.customer_id}, suspending`);
    
    await db.query(
      `UPDATE dunning_attempts SET status = 'failed', updated_at = NOW() WHERE id = $1`,
      [attempt.id]
    );
    
    if (DUNNING_CONFIG.suspendAfterFailures) {
      await suspendSubscription(attempt.customer_id, attempt.subscription_id);
      await emitSuspensionEmail(attempt.customer_id);
    }
    
    return {
      id: attempt.id,
      customerId: attempt.customer_id,
      invoiceId: attempt.invoice_id,
      success: false,
      message: 'Max retries reached, subscription suspended',
      nextAction: 'subscription_suspended',
    };
  }
  
  // Schedule next retry
  const daysUntilNextRetry = DUNNING_CONFIG.retryScheduleDays[nextAttemptNumber] || 7;
  const nextRetry = new Date();
  nextRetry.setDate(nextRetry.getDate() + daysUntilNextRetry);
  
  await db.query(
    `UPDATE dunning_attempts 
     SET attempt_number = $1,
         next_retry_at = $2,
         status = 'pending',
         updated_at = NOW()
     WHERE id = $3`,
    [nextAttemptNumber, nextRetry, attempt.id]
  );
  
  // Send retry notification email
  await emitRetryEmail(attempt.customer_id, attempt.invoice_id, nextAttemptNumber, daysUntilNextRetry);
  
  console.log(`[Dunning] Scheduled retry ${nextAttemptNumber} for customer ${attempt.customer_id} on ${nextRetry.toISOString()}`);
  
  return {
    id: attempt.id,
    customerId: attempt.customer_id,
    invoiceId: attempt.invoice_id,
    success: false,
    message: `Retry scheduled for ${nextRetry.toISOString()}`,
    nextAction: 'retry_scheduled',
  };
}

/**
 * Mark dunning as successful
 */
async function markDunningSuccess(attempt: DunningAttempt): Promise<void> {
  await db.query(
    `UPDATE dunning_attempts SET status = 'succeeded', updated_at = NOW() WHERE id = $1`,
    [attempt.id]
  );
  
  // Reactivate subscription
  await db.query(
    `UPDATE subscriptions 
     SET status = 'active',
         failed_payment_count = 0
     WHERE stripe_subscription_id = $1`,
    [attempt.subscription_id]
  );
  
  // Reactivate API keys
  await db.query(
    `UPDATE api_keys 
     SET status = 'active',
         suspended_at = NULL
     WHERE user_id = $1 AND status = 'suspended'`,
    [attempt.customer_id]
  );
}

/**
 * Suspend subscription after failed dunning
 */
async function suspendSubscription(customerId: string, subscriptionId: string): Promise<void> {
  console.log(`[Dunning] Suspending subscription ${subscriptionId}`);
  
  // Pause subscription in Stripe
  try {
    await stripe.subscriptions.update(subscriptionId, {
      pause_collection: {
        behavior: 'void',
      },
    });
  } catch (error: any) {
    console.error('[Dunning] Error pausing subscription in Stripe:', error.message);
  }
  
  // Update in database
  await db.query(
    `UPDATE subscriptions 
     SET status = 'suspended',
         suspended_at = NOW()
     WHERE stripe_subscription_id = $1`,
    [subscriptionId]
  );
  
  // Suspend API keys
  await db.query(
    `UPDATE api_keys 
     SET status = 'suspended',
         suspended_at = NOW()
     WHERE user_id = $1 AND status = 'active'`,
    [customerId]
  );
  
  console.log(`[Dunning] Subscription and API access suspended for customer ${customerId}`);
}

/**
 * Emit email events
 */
async function emitSuccessEmail(customerId: string, invoice: Stripe.Invoice): Promise<void> {
  const amountPaid = (invoice.amount_paid / 100).toFixed(2);
  await db.query(
    `INSERT INTO events (event_type, data, created_at)
     VALUES ($1, $2, NOW())`,
    [
      'email.payment_succeeded_after_retry',
      JSON.stringify({
        customer_id: customerId,
        invoice_id: invoice.id,
        amount_paid: amountPaid,
        currency: invoice.currency,
      }),
    ]
  );
}

async function emitRetryEmail(
  customerId: string,
  invoiceId: string,
  attemptNumber: number,
  daysUntilRetry: number
): Promise<void> {
  await db.query(
    `INSERT INTO events (event_type, data, created_at)
     VALUES ($1, $2, NOW())`,
    [
      'email.payment_retry',
      JSON.stringify({
        customer_id: customerId,
        invoice_id: invoiceId,
        attempt_number: attemptNumber,
        days_until_retry: daysUntilRetry,
        max_retries: DUNNING_CONFIG.maxRetries,
      }),
    ]
  );
}

async function emitSuspensionEmail(customerId: string): Promise<void> {
  await db.query(
    `INSERT INTO events (event_type, data, created_at)
     VALUES ($1, $2, NOW())`,
    [
      'email.subscription_suspended',
      JSON.stringify({
        customer_id: customerId,
        reactivation_url: 'https://console.apexmediation.ee/billing/reactivate',
      }),
    ]
  );
}

/**
 * Log results to audit trail
 */
async function logAuditTrail(results: DunningResult[]): Promise<void> {
  const successCount = results.filter((r) => r.success).length;
  const failedCount = results.filter((r) => !r.success).length;
  const suspendedCount = results.filter((r) => r.nextAction === 'subscription_suspended').length;
  
  await db.query(
    `INSERT INTO billing_audit (event_type, data, created_at)
     VALUES ($1, $2, NOW())`,
    [
      'dunning_cron_completed',
      JSON.stringify({
        total_processed: results.length,
        successful: successCount,
        failed: failedCount,
        subscriptions_suspended: suspendedCount,
        timestamp: new Date().toISOString(),
      }),
    ]
  );
}

/**
 * Main entry point
 */
async function main(): Promise<void> {
  const startTime = Date.now();
  console.log(`[${new Date().toISOString()}] Starting dunning retry cron job...`);
  
  try {
    // Fetch due dunning attempts
    const attempts = await fetchDueDunningAttempts();
    console.log(`Found ${attempts.length} dunning attempts due for retry`);
    
    if (attempts.length === 0) {
      console.log('No dunning attempts to process');
      return;
    }
    
    const results: DunningResult[] = [];
    
    // Process each attempt sequentially to avoid overwhelming Stripe
    for (const attempt of attempts) {
      try {
        const result = await retryPayment(attempt);
        results.push(result);
        
        // Small delay between retries to be nice to Stripe
        await setTimeout(500);
      } catch (error: any) {
        console.error(`[Dunning] Error processing attempt ${attempt.id}:`, error.message);
        results.push({
          id: attempt.id,
          customerId: attempt.customer_id,
          invoiceId: attempt.invoice_id,
          success: false,
          message: error.message,
        });
      }
    }
    
    // Log audit trail
    await logAuditTrail(results);
    
    const duration = Date.now() - startTime;
    console.log(`\n[${new Date().toISOString()}] Dunning retry cron completed in ${duration}ms`);
    console.log(`  Processed: ${results.length}`);
    console.log(`  Successful: ${results.filter((r) => r.success).length}`);
    console.log(`  Failed: ${results.filter((r) => !r.success).length}`);
    console.log(`  Suspended: ${results.filter((r) => r.nextAction === 'subscription_suspended').length}`);
    
    process.exit(0);
  } catch (error: any) {
    console.error('Fatal error in dunning retry cron:', error);
    process.exit(1);
  } finally {
    await db.end();
  }
}

// Run if executed directly
main().catch(console.error);
