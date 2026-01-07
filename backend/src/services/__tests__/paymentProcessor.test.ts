/**
 * Payment Processor Tests (Stripe Only)
 * Uses simulation mode (no STRIPE_SECRET_KEY) for testing
 */

import {
  PaymentProcessor,
  PayoutLedgerService,
  paymentProcessor,
  ledgerService,
  PayoutRequest,
} from '../payoutProcessor';
import { query, getClient } from '../../utils/postgres';

// Store original env to restore after tests
const originalStripeKey = process.env.STRIPE_SECRET_KEY;

beforeAll(() => {
  // Clear Stripe key to use simulation mode
  delete process.env.STRIPE_SECRET_KEY;
});

afterAll(() => {
  // Restore original env
  if (originalStripeKey) {
    process.env.STRIPE_SECRET_KEY = originalStripeKey;
  }
});

describe('PaymentProcessor', () => {
  let processor: PaymentProcessor;

  beforeEach(() => {
    processor = new PaymentProcessor();
  });

  describe('processPayout', () => {
    const validPayoutRequest: PayoutRequest = {
      payoutId: `payout-${Date.now()}`,
      publisherId: `publisher-${Date.now()}`,
      amount: 500,
      currency: 'USD',
      method: 'wire',
      recipientEmail: 'test@example.com',
    };

    it('should process payout via Stripe simulation', async () => {
      const result = await processor.processPayout(validPayoutRequest);

      expect(result.success).toBe(true);
      expect(result.provider).toBe('stripe');
      expect(result.transactionId).toContain('stripe_sim_');
    });

    it('should rate limit after 5 payouts per minute per publisher', async () => {
      const publisherId = `rate-test-${Date.now()}`;
      const results = [];
      
      for (let i = 0; i < 6; i++) {
        const r = await processor.processPayout({
          ...validPayoutRequest,
          payoutId: `payout-rate-${i}-${Date.now()}`,
          publisherId,
        });
        results.push(r);
      }

      const successes = results.filter(r => r.success).length;
      const rateLimited = results.find(r => r.error?.includes('Rate limited'));
      
      expect(successes).toBe(5);
      expect(rateLimited).toBeTruthy();
    });
  });

  describe('retryFailedPayouts', () => {
    it('should return processed/failed counts', async () => {
      const result = await processor.retryFailedPayouts();

      expect(result).toHaveProperty('processed');
      expect(result).toHaveProperty('failed');
      expect(typeof result.processed).toBe('number');
      expect(typeof result.failed).toBe('number');
    });
  });
});

describe('PayoutLedgerService', () => {
  let service: PayoutLedgerService;

  beforeEach(() => {
    service = new PayoutLedgerService();
  });

  // Note: These tests require database connectivity
  // In CI, you would need to have a test database configured
  
  describe('getLedgerEntries', () => {
    it('should return array (empty or with entries depending on DB state)', async () => {
      const result = await service.getLedgerEntries(`test-${Date.now()}`);
      expect(Array.isArray(result)).toBe(true);
    });
  });
});

describe('Payment Provider Integration', () => {
  it('should export singleton instances', () => {
    expect(paymentProcessor).toBeInstanceOf(PaymentProcessor);
    expect(ledgerService).toBeInstanceOf(PayoutLedgerService);
  });
});
