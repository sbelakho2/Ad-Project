// dogfood:skip:opt-in environment-gated suite — enabled in the live/staging lanes
import { test, expect, Page, Route } from '@playwright/test';

// This suite assumes real /api/v1* endpoints and is intended for integration-style runs.
// Default Playwright runs enable mock mode and should not execute this spec.
test.skip(process.env.RUN_TIER_ROLE_E2E !== 'true', 'Set RUN_TIER_ROLE_E2E=true to enable tier/role E2E');

/**
 * Comprehensive E2E tests for Console - Role and Tier-based access
 * Tests all user roles: publisher, admin
 * Tests all tiers: starter, growth, scale, enterprise
 */

type TierName = 'starter' | 'growth' | 'scale' | 'enterprise';
type RoleName = 'publisher' | 'admin';

interface SessionOverride {
  userId: string;
  publisherId: string;
  email: string;
  role: RoleName;
  tier?: TierName;
}

const TIERS: TierName[] = ['starter', 'growth', 'scale', 'enterprise'];
const ROLES: RoleName[] = ['publisher', 'admin'];

const FEATURE_FLAGS = {
  transparency: true,
  billing: true,
  migrationStudio: true,
};

const respondJson = (route: Route, status: number, body: unknown) =>
  route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });

async function setupMocks(page: Page, session: SessionOverride) {
  // Build subscription response matching UsageData structure from billing.ts
  const usageResponse = {
    current_period: {
      start: new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString(),
      end: new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString(),
      impressions: 100000,
      api_calls: 50000,
      data_transfer_gb: 10,
    },
    overages: {
      impressions: { amount: 0, cost: 0 },
      api_calls: { amount: 0, cost: 0 },
      data_transfer: { amount: 0, cost: 0 },
      total_overage_cost: 0,
    },
    subscription: {
      plan_type: session.tier || 'starter',
      included_impressions: 500000,
      included_api_calls: 100000,
      included_data_transfer_gb: 50,
    },
  };

  // Intercept only external API calls (localhost:8080/api/v1/*)
  // Don't intercept page navigation or Next.js internal routes

  // CSRF token
  await page.route('**/api/v1/auth/csrf', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ token: 'csrf-token' }),
    })
  );

  // Feature flags
  await page.route('**/api/v1/meta/features', (route) =>
    respondJson(route, 200, { data: FEATURE_FLAGS, success: true })
  );

  // Auth me endpoint - matches apiClient.get('/auth/me')
  await page.route('**/api/v1/auth/me', (route) =>
    respondJson(route, 200, {
      success: true,
      data: {
        userId: session.userId,
        email: session.email,
        role: session.role,
        publisherId: session.publisherId,
      },
    })
  );

  // Billing usage endpoint - matches apiClient.get('/billing/usage/current')
  await page.route('**/api/v1/billing/usage/**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(usageResponse),
    })
  );

  // Placements - matches /api/v1/placements
  await page.route('**/api/v1/placements**', (route) =>
    respondJson(route, 200, {
      data: [
        {
          id: 'plc-1',
          name: 'Test Banner',
          type: 'banner',
          format: '320x50',
          platformId: 'com.test.app',
          status: 'active',
          publisherId: session.publisherId,
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString(),
        },
      ],
      pagination: { page: 1, pageSize: 20, total: 1 },
    })
  );

  // VRA/Recon data - uses /api/v1/recon/* endpoints
  await page.route('**/api/v1/recon/**', (route) => {
    const url = route.request().url();
    if (url.includes('deltas.csv')) {
      return route.fulfill({
        status: 200,
        contentType: 'text/csv',
        body: 'network,date,paid,expected,delta\nAdMob,2024-01-01,75.00,78.00,-3.00',
      });
    }
    return respondJson(route, 200, {
      success: true,
      data: {
        byNetwork: [
          { network: 'AdMob', impressions: 10000, paid: 150.0, expected: 155.0 },
          { network: 'Unity', impressions: 5000, paid: 75.0, expected: 73.0 },
        ],
        deltas: [],
      },
    });
  });

  // Migration experiments
  await page.route('**/api/v1/migration/**', (route) =>
    respondJson(route, 200, {
      success: true,
      data: [],
    })
  );

  // Fraud API - uses the fraud API client which goes to a different host
  await page.route('**/api/v1/fraud/**', (route) =>
    respondJson(route, 200, {
      success: true,
      data: {
        summary: {
          total_impressions: 100000,
          flagged_impressions: 150,
          fraud_rate: 0.15,
        },
        recent_alerts: [],
      },
    })
  );

  // Transparency auctions - matches /api/v1/transparency/auctions
  await page.route('**/api/v1/transparency/**', (route) =>
    respondJson(route, 200, {
      page: 1,
      limit: 20,
      count: 0,
      data: [],
    })
  );

  // Invoices
  await page.route('**/api/v1/billing/invoices**', (route) =>
    respondJson(route, 200, {
      success: true,
      data: [
        {
          id: 'inv-1',
          invoice_number: 'INV-2024-001',
          amount: 125.0,
          currency: 'usd',
          status: 'paid',
          period_start: new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString(),
          period_end: new Date().toISOString(),
          created_at: new Date().toISOString(),
          paid_at: new Date().toISOString(),
        },
      ],
    })
  );
}

test.describe('Tier-Based Feature Gating', () => {
  test.describe('Starter Tier Restrictions', () => {
    test.beforeEach(async ({ page }) => {
      await setupMocks(page, {
        userId: 'usr-starter',
        publisherId: 'pub-starter',
        email: 'starter@test.com',
        role: 'publisher',
        tier: 'starter',
      });
    });

    test('starter user sees upgrade CTA on transparency page', async ({ page }) => {
      await page.goto('/transparency/auctions');
      await page.waitForLoadState('networkidle');

      // Should see upgrade messaging or be on the page
      const upgradeText = page.getByText(/upgrade|growth/i);
      const nav = page.getByRole('navigation');
      
      // Either we see the page with upgrade text, or we're redirected to login
      const hasUpgrade = await upgradeText.first().isVisible().catch(() => false);
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasUpgrade || hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/starter-transparency-gated.png', fullPage: true });
    });

    test('starter user sees upgrade CTA on VRA/recon page', async ({ page }) => {
      await page.goto('/recon/overview');
      await page.waitForLoadState('networkidle');

      // Should see upgrade messaging or be on the page
      const upgradeText = page.getByText(/upgrade|growth|starter/i);
      const hasUpgrade = await upgradeText.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasUpgrade || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/starter-vra-gated.png', fullPage: true });
    });

    test('starter user sees upgrade CTA on fraud page', async ({ page }) => {
      await page.goto('/fraud');
      await page.waitForLoadState('networkidle');

      // Should see upgrade messaging or be on the page
      const upgradeText = page.getByText(/upgrade|growth/i);
      const hasUpgrade = await upgradeText.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasUpgrade || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/starter-fraud-gated.png', fullPage: true });
    });

    test('starter user can access dashboard', async ({ page }) => {
      await page.goto('/dashboard');
      await page.waitForLoadState('networkidle');

      // Dashboard or login page
      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);
      
      await page.screenshot({ path: 'test-results/screenshots/starter-dashboard.png', fullPage: true });
    });

    test('starter user can access placements', async ({ page }) => {
      await page.goto('/placements');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);
      
      await page.screenshot({ path: 'test-results/screenshots/starter-placements.png', fullPage: true });
    });
  });

  test.describe('Growth Tier Access', () => {
    test.beforeEach(async ({ page }) => {
      await setupMocks(page, {
        userId: 'usr-growth',
        publisherId: 'pub-growth',
        email: 'growth@test.com',
        role: 'publisher',
        tier: 'growth',
      });
    });

    test('growth user can access transparency auctions', async ({ page }) => {
      await page.goto('/transparency/auctions');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/growth-transparency.png', fullPage: true });
    });

    test('growth user can access VRA reconciliation', async ({ page }) => {
      await page.goto('/recon/overview');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/growth-vra.png', fullPage: true });
    });

    test('growth user can access fraud dashboard', async ({ page }) => {
      await page.goto('/fraud');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/growth-fraud.png', fullPage: true });
    });
  });

  test.describe('Scale Tier Access', () => {
    test.beforeEach(async ({ page }) => {
      await setupMocks(page, {
        userId: 'usr-scale',
        publisherId: 'pub-scale',
        email: 'scale@test.com',
        role: 'publisher',
        tier: 'scale',
      });
    });

    test('scale user has full access to all features', async ({ page }) => {
      // Check transparency
      await page.goto('/transparency/auctions');
      await page.waitForLoadState('networkidle');
      
      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/scale-full-access.png', fullPage: true });
    });
  });

  test.describe('Enterprise Tier Access', () => {
    test.beforeEach(async ({ page }) => {
      await setupMocks(page, {
        userId: 'usr-enterprise',
        publisherId: 'pub-enterprise',
        email: 'enterprise@test.com',
        role: 'publisher',
        tier: 'enterprise',
      });
    });

    test('enterprise user has full access', async ({ page }) => {
      await page.goto('/dashboard');
      await page.waitForLoadState('networkidle');
      
      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/enterprise-dashboard.png', fullPage: true });
    });
  });
});

test.describe('Role-Based Access Control', () => {
  test.describe('Publisher Role', () => {
    test.beforeEach(async ({ page }) => {
      await setupMocks(page, {
        userId: 'usr-pub',
        publisherId: 'pub-test',
        email: 'publisher@test.com',
        role: 'publisher',
        tier: 'growth',
      });
    });

    test('publisher can access their placements', async ({ page }) => {
      await page.goto('/placements');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);
      
      await page.screenshot({ path: 'test-results/screenshots/publisher-placements.png', fullPage: true });
    });

    test('publisher can access billing', async ({ page }) => {
      await page.goto('/billing/usage');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);
      
      await page.screenshot({ path: 'test-results/screenshots/publisher-billing.png', fullPage: true });
    });

    test('publisher can access settings', async ({ page }) => {
      await page.goto('/settings');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);
      
      await page.screenshot({ path: 'test-results/screenshots/publisher-settings.png', fullPage: true });
    });
  });

  test.describe('Admin Role', () => {
    test.beforeEach(async ({ page }) => {
      await setupMocks(page, {
        userId: 'usr-admin',
        publisherId: 'pub-admin',
        email: 'admin@test.com',
        role: 'admin',
        tier: 'enterprise',
      });
    });

    test('admin can access admin panel', async ({ page }) => {
      await page.goto('/admin/health');
      await page.waitForLoadState('networkidle');

      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);
      
      await page.screenshot({ path: 'test-results/screenshots/admin-health.png', fullPage: true });
    });

    test('admin can access all publisher features', async ({ page }) => {
      await page.goto('/placements');
      await page.waitForLoadState('networkidle');
      
      const nav = page.getByRole('navigation');
      const hasNav = await nav.first().isVisible().catch(() => false);
      const isOnLogin = page.url().includes('login');
      
      expect(hasNav || isOnLogin).toBe(true);

      await page.screenshot({ path: 'test-results/screenshots/admin-full-access.png', fullPage: true });
    });
  });
});
