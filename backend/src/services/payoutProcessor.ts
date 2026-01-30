import {
  fetchPayoutHistory,
  fetchUpcomingPayouts,
  fetchPayoutSettings,
  upsertPayoutSettings,
  countPayoutHistory,
  PayoutSettingsInput,
} from '../repositories/payoutRepository';
import { query, getClient } from '../utils/postgres';
import logger from '../utils/logger';
import Stripe from 'stripe';

// ========================================
// Payment Provider Types
// ========================================

export type PaymentProvider = 'stripe';

export interface PaymentProviderConfig {
  name: PaymentProvider;
  enabled: boolean;
  apiKey?: string;
}

export interface PayoutRequest {
  payoutId: string;
  publisherId: string;
  amount: number;
  currency: string;
  method: string;
  recipientEmail?: string;
  recipientAccount?: string;
  stripeConnectedAccountId?: string;
}

export interface PayoutResult {
  success: boolean;
  provider: PaymentProvider;
  transactionId?: string;
  error?: string;
  retryable: boolean;
}

export interface LedgerEntry {
  id: string;
  payoutId: string;
  publisherId: string;
  amount: number;
  currency: string;
  type: 'debit' | 'credit';
  provider: PaymentProvider;
  createdAt: Date;
}

// ========================================
// Payment Provider Configuration
// ========================================

const PAYMENT_PROVIDER: PaymentProviderConfig = {
  name: 'stripe' as const,
  enabled: true,
  apiKey: process.env.STRIPE_SECRET_KEY,
};

// ========================================
// Stripe Client Singleton
// ========================================

let stripeClient: Stripe | null = null;

const getStripeClient = (): Stripe | null => {
  if (stripeClient) return stripeClient;

  const secretKey = process.env.STRIPE_SECRET_KEY;
  if (!secretKey) {
    return null;
  }

  stripeClient = new Stripe(secretKey, {
    apiVersion: '2025-10-29.clover',
  });

  return stripeClient;
};

// ========================================
// Stripe Payment Provider Implementation
// ========================================

class StripeProvider {
  async sendPayout(request: PayoutRequest): Promise<PayoutResult> {
    try {
      logger.info('Processing payout via Stripe', {
        payoutId: request.payoutId,
        amount: request.amount,
        currency: request.currency,
      });

      // PRODUCTION SAFETY: Fail-closed when API key is not configured
      if (!process.env.STRIPE_SECRET_KEY) {
        if (process.env.NODE_ENV === 'production') {
          logger.error('Stripe API key not configured in production - rejecting payout', {
            payoutId: request.payoutId,
          });
          return {
            success: false,
            provider: 'stripe',
            error: 'Payment provider not configured. Contact support.',
            retryable: false,
          };
        }
        logger.warn('Stripe API key not configured (non-production mode)');
      }

      const stripe = getStripeClient();
      
      // In non-production without Stripe key, simulate success for testing
      if (!stripe) {
        if (process.env.NODE_ENV !== 'production') {
          const simulatedTransactionId = `stripe_sim_${Date.now()}_${Math.random().toString(36).substring(7)}`;
          logger.info('Stripe payout simulated (non-production)', {
            payoutId: request.payoutId,
            transactionId: simulatedTransactionId,
          });
          return {
            success: true,
            provider: 'stripe',
            transactionId: simulatedTransactionId,
            retryable: false,
          };
        }
        return {
          success: false,
          provider: 'stripe',
          error: 'Payment provider not configured. Contact support.',
          retryable: false,
        };
      }

      // For Stripe Connect payouts, we create a transfer to the connected account
      if (request.stripeConnectedAccountId) {
        const transfer = await stripe.transfers.create({
          amount: Math.round(request.amount * 100), // Convert to cents
          currency: request.currency.toLowerCase(),
          destination: request.stripeConnectedAccountId,
          metadata: {
            payout_id: request.payoutId,
            publisher_id: request.publisherId,
          },
        });

        logger.info('Stripe transfer successful', {
          payoutId: request.payoutId,
          transactionId: transfer.id,
        });

        return {
          success: true,
          provider: 'stripe',
          transactionId: transfer.id,
          retryable: false,
        };
      }

      // For direct payouts to platform balance (e.g., bank payout)
      const payout = await stripe.payouts.create({
        amount: Math.round(request.amount * 100), // Convert to cents
        currency: request.currency.toLowerCase(),
        metadata: {
          payout_id: request.payoutId,
          publisher_id: request.publisherId,
        },
      });

      logger.info('Stripe payout successful', {
        payoutId: request.payoutId,
        transactionId: payout.id,
      });

      return {
        success: true,
        provider: 'stripe',
        transactionId: payout.id,
        retryable: false,
      };
    } catch (error) {
      const stripeError = error as Stripe.errors.StripeError;
      logger.error('Stripe payout failed', { 
        error: stripeError.message, 
        code: stripeError.code,
        payoutId: request.payoutId,
      });
      
      // Determine if error is retryable
      const retryable = stripeError.type === 'StripeConnectionError' ||
                        stripeError.type === 'StripeAPIError' ||
                        stripeError.code === 'rate_limit';
      
      return {
        success: false,
        provider: 'stripe',
        error: stripeError.message || 'Unknown Stripe error',
        retryable,
      };
    }
  }
}

// ========================================
// Payment Provider Instance
// ========================================

const stripeProvider = new StripeProvider();

// ========================================
// Ledger Service
// ========================================

export class PayoutLedgerService {
  /**
   * Create double-entry ledger entries for a payout
   * 
   * CRITICAL: Uses a database transaction to ensure atomicity.
   * Both debit and credit entries must succeed together or both fail.
   * This maintains double-entry accounting integrity.
   */
  async createLedgerEntries(
    payoutId: string,
    publisherId: string,
    amount: number,
    currency: string,
    provider: PaymentProvider
  ): Promise<{ debitId: string; creditId: string }> {
    const client = await getClient();
    
    try {
      await client.query('BEGIN');
      
      // Debit entry (money leaving platform)
      const debitResult = await client.query<{ id: string }>(
        `INSERT INTO payout_ledger (payout_id, publisher_id, amount, currency, type, provider, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())
         RETURNING id`,
        [payoutId, publisherId, amount, currency, 'debit', provider]
      );

      // Credit entry (publisher receiving money)
      const creditResult = await client.query<{ id: string }>(
        `INSERT INTO payout_ledger (payout_id, publisher_id, amount, currency, type, provider, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())
         RETURNING id`,
        [payoutId, publisherId, amount, currency, 'credit', provider]
      );

      await client.query('COMMIT');

      logger.info('Ledger entries created', {
        payoutId,
        debitId: debitResult.rows[0].id,
        creditId: creditResult.rows[0].id,
      });

      return {
        debitId: debitResult.rows[0].id,
        creditId: creditResult.rows[0].id,
      };
    } catch (error) {
      await client.query('ROLLBACK');
      logger.error('Failed to create ledger entries', { error, payoutId });
      throw new Error('Ledger entry creation failed');
    } finally {
      client.release();
    }
  }

  /**
   * Validate ledger balance (should always be zero for double-entry)
   */
  async validateLedgerBalance(payoutId: string): Promise<boolean> {
    try {
      const result = await query<{ balance: number }>(
        `SELECT SUM(
           CASE 
             WHEN type = 'debit' THEN -amount
             WHEN type = 'credit' THEN amount
             ELSE 0
           END
         ) as balance
         FROM payout_ledger
         WHERE payout_id = $1`,
        [payoutId]
      );

      const balance = Number(result.rows[0]?.balance || 0);
      const isBalanced = Math.abs(balance) < 0.01; // Allow for floating point errors

      if (!isBalanced) {
        logger.error('Ledger balance validation failed', {
          payoutId,
          balance,
        });
      }

      return isBalanced;
    } catch (error) {
      logger.error('Failed to validate ledger balance', { error, payoutId });
      return false;
    }
  }

  /**
   * Get ledger entries for a payout
   */
  async getLedgerEntries(payoutId: string): Promise<LedgerEntry[]> {
    try {
      const result = await query<{
        id: string;
        payout_id: string;
        publisher_id: string;
        amount: number;
        currency: string;
        type: 'debit' | 'credit';
        provider: PaymentProvider;
        created_at: Date;
      }>(
        `SELECT id, payout_id, publisher_id, amount, currency, type, provider, created_at
         FROM payout_ledger
         WHERE payout_id = $1
         ORDER BY created_at ASC`,
        [payoutId]
      );

      return result.rows.map((row) => ({
        id: row.id,
        payoutId: row.payout_id,
        publisherId: row.publisher_id,
        amount: row.amount,
        currency: row.currency,
        type: row.type,
        provider: row.provider,
        createdAt: row.created_at,
      }));
    } catch (error) {
      logger.error('Failed to get ledger entries', { error, payoutId });
      return [];
    }
  }
}

export const ledgerService = new PayoutLedgerService();

// ========================================
// Payment Processor (Stripe Only)
// ========================================

export class PaymentProcessor {
  private static rateLimiter: Map<string, { count: number; resetAt: number }> = new Map();
  private static readonly RATE_LIMIT_WINDOW_MS = 60_000; // 1 minute
  private static readonly RATE_LIMIT_MAX = 5; // 5 payouts per publisher per minute

  private isRateLimited(publisherId: string): boolean {
    const now = Date.now();
    const key = publisherId;
    const entry = PaymentProcessor.rateLimiter.get(key);
    if (!entry || entry.resetAt <= now) {
      PaymentProcessor.rateLimiter.set(key, { count: 1, resetAt: now + PaymentProcessor.RATE_LIMIT_WINDOW_MS });
      return false;
    }
    if (entry.count >= PaymentProcessor.RATE_LIMIT_MAX) {
      return true;
    }
    entry.count += 1;
    PaymentProcessor.rateLimiter.set(key, entry);
    return false;
  }

  /**
   * Process payout via Stripe
   */
  async processPayout(request: PayoutRequest): Promise<PayoutResult> {
    // Simple per-publisher rate limiter to avoid bursts
    if (this.isRateLimited(request.publisherId)) {
      logger.warn('Payout rate limit exceeded for publisher', { publisherId: request.publisherId });
      return {
        success: false,
        provider: 'stripe',
        error: 'Rate limited: too many payout requests',
        retryable: true,
      };
    }

    logger.info('Starting payout processing', {
      payoutId: request.payoutId,
      publisherId: request.publisherId,
      amount: request.amount,
      currency: request.currency,
    });

    if (!PAYMENT_PROVIDER.enabled) {
      logger.error('Payment provider not enabled');
      return {
        success: false,
        provider: 'stripe',
        error: 'Payment provider not available',
        retryable: false,
      };
    }

    try {
      const result = await stripeProvider.sendPayout(request);

      if (result.success) {
        // In simulation mode (non-production without Stripe key), skip DB writes
        if (result.transactionId?.startsWith('stripe_sim_')) {
          logger.info('Payout processed successfully (simulation mode)', {
            payoutId: request.payoutId,
            provider: 'stripe',
            transactionId: result.transactionId,
          });
          return result;
        }

        // Perform ledger creation + status update atomically
        const client = await getClient();
        try {
          await client.query('BEGIN');

          // Debit entry (money leaving platform)
          await client.query(
            `INSERT INTO payout_ledger (payout_id, publisher_id, amount, currency, type, provider, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())`,
            [request.payoutId, request.publisherId, request.amount, request.currency, 'debit', 'stripe']
          );

          // Credit entry (publisher receiving money)
          await client.query(
            `INSERT INTO payout_ledger (payout_id, publisher_id, amount, currency, type, provider, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())`,
            [request.payoutId, request.publisherId, request.amount, request.currency, 'credit', 'stripe']
          );

          // Validate ledger balance within tx
          const balRes = await client.query<{ balance: number }>(
            `SELECT SUM(
               CASE WHEN type = 'debit' THEN -amount WHEN type = 'credit' THEN amount ELSE 0 END
             ) as balance FROM payout_ledger WHERE payout_id = $1`,
            [request.payoutId]
          );
          const balance = Number(balRes.rows[0]?.balance ?? 0);
          if (Math.abs(balance) >= 0.01) {
            throw new Error(`Ledger not balanced (balance=${balance})`);
          }

          // Update payout status
          await client.query(
            `UPDATE payouts SET status = $1, provider = $2, transaction_id = $3, processed_at = NOW(), updated_at = NOW() WHERE id = $4`,
            ['processed', 'stripe', result.transactionId ?? null, request.payoutId]
          );

          await client.query('COMMIT');

          logger.info('Payout processed successfully', {
            payoutId: request.payoutId,
            provider: 'stripe',
            transactionId: result.transactionId,
          });

          return result;
        } catch (e) {
          try { await client.query('ROLLBACK'); } catch {}
          logger.error('Failed to finalize payout atomically, rolled back', {
            payoutId: request.payoutId,
            provider: 'stripe',
            error: e,
          });
          return {
            success: false,
            provider: 'stripe',
            error: 'Failed to record payout in ledger',
            retryable: true,
          };
        } finally {
          client.release();
        }
      }

      // Return the failure result
      return result;

    } catch (error) {
      logger.error('Unexpected error during payout', {
        payoutId: request.payoutId,
        provider: 'stripe',
        error,
      });

      await this.updatePayoutStatus(request.payoutId, 'failed', 'stripe', null);

      return {
        success: false,
        provider: 'stripe',
        error: error instanceof Error ? error.message : 'Unknown error',
        retryable: true,
      };
    }
  }

  /**
   * Update payout status in database
   */
  private async updatePayoutStatus(
    payoutId: string,
    status: string,
    provider: PaymentProvider | null,
    transactionId: string | null | undefined
  ): Promise<void> {
    try {
      await query(
        `UPDATE payouts
         SET status = $1,
             provider = $2,
             transaction_id = $3,
             processed_at = CASE WHEN $1 = 'processed' THEN NOW() ELSE NULL END,
             updated_at = NOW()
         WHERE id = $4`,
        [status, provider, transactionId, payoutId]
      );
    } catch (error) {
      logger.error('Failed to update payout status', { error, payoutId });
    }
  }

  /**
   * Retry failed payouts
   */
  async retryFailedPayouts(): Promise<{ processed: number; failed: number }> {
    try {
      // Get all failed payouts from last 24 hours
      const result = await query<{
        id: string;
        publisher_id: string;
        amount: number;
        currency: string;
        method: string;
      }>(
        `SELECT id, publisher_id, amount, currency, method
         FROM payouts
         WHERE status = 'failed'
           AND created_at > NOW() - INTERVAL '24 hours'
         LIMIT 100`
      );

      let processed = 0;
      let failed = 0;

      for (const payout of result.rows) {
        const request: PayoutRequest = {
          payoutId: payout.id,
          publisherId: payout.publisher_id,
          amount: payout.amount,
          currency: payout.currency,
          method: payout.method,
        };

        const payoutResult = await this.processPayout(request);
        
        if (payoutResult.success) {
          processed++;
        } else {
          failed++;
        }
      }

      logger.info('Completed failed payout retry', { processed, failed });

      return { processed, failed };
    } catch (error) {
      logger.error('Failed to retry payouts', { error });
      return { processed: 0, failed: 0 };
    }
  }
}

export const paymentProcessor = new PaymentProcessor();

// ========================================
// Existing API Functions
// ========================================

export const listPayoutHistory = async (
  publisherId: string,
  page = 1,
  pageSize = 10
) => {
  const safePage = Math.max(1, page);
  const safeSize = Math.min(Math.max(1, pageSize), 200);
  const offset = (safePage - 1) * safeSize;

  const [rows, total] = await Promise.all([
    fetchPayoutHistory(publisherId, safeSize, offset),
    countPayoutHistory(publisherId),
  ]);

  return {
    items: rows,
    total,
    page: safePage,
    pageSize: safeSize,
    hasMore: safePage * safeSize < total,
  };
};

export const getUpcomingPayouts = async (publisherId: string) => {
  return fetchUpcomingPayouts(publisherId);
};

export const getPayoutSettings = async (publisherId: string) => {
  return fetchPayoutSettings(publisherId);
};

export const updatePayoutSettings = async (
  publisherId: string,
  settings: PayoutSettingsInput
) => {
  return upsertPayoutSettings(publisherId, settings);
};
