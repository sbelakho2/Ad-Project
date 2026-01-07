import { PaymentProcessor } from '../payoutProcessor';

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

describe('PaymentProcessor behavior', () => {
  let processor: PaymentProcessor;

  beforeEach(() => {
    processor = new PaymentProcessor();
  });

  test('rate limiting caps at 5 payouts per minute per publisher', async () => {
    const publisherId = `pub-rate-${Date.now()}`;
    const results: Array<{ success: boolean; error?: string }> = [];

    for (let i = 0; i < 6; i++) {
      const r = await processor.processPayout({
        publisherId,
        payoutId: `p-${i}-${Date.now()}`,
        amount: 100,
        currency: 'USD',
        method: 'wire',
      });
      results.push({ success: r.success, error: r.error });
    }

    const successes = results.filter(r => r.success).length;
    const rateLimited = results.find(r => r.error?.includes('Rate limited'));
    expect(successes).toBe(5);
    expect(rateLimited).toBeTruthy();
  });

  test('fails-closed in production when Stripe API key not configured', async () => {
    const originalNodeEnv = process.env.NODE_ENV;

    try {
      // Set to production mode
      process.env.NODE_ENV = 'production';

      // Create fresh processor instance
      const prodProcessor = new PaymentProcessor();

      const result = await prodProcessor.processPayout({
        publisherId: `pub-prod-${Date.now()}`,
        payoutId: `prod-fail-closed-${Date.now()}`,
        amount: 100,
        currency: 'USD',
        method: 'wire',
      });

      // Should fail because no API key is configured in production
      expect(result.success).toBe(false);
      expect(result.error).toContain('Payment provider not configured');
    } finally {
      // Restore original env
      process.env.NODE_ENV = originalNodeEnv;
    }
  });
});
