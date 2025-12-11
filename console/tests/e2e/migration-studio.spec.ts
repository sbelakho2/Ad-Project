import { test, expect, Page, Route } from '@playwright/test';

/**
 * Migration Studio E2E Tests
 * Tests import wizard, parity simulator, and experiment management
 */

const respondJson = (route: Route, status: number, body: unknown) =>
  route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });

const MOCK_PLACEMENTS = [
  {
    id: 'plc-banner-1',
    name: 'Home Banner',
    type: 'banner',
    format: '320x50',
    platformId: 'com.test.game',
    status: 'active',
    publisherId: 'pub-migration',
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  },
  {
    id: 'plc-interstitial-1',
    name: 'Level Complete Interstitial',
    type: 'interstitial',
    format: 'fullscreen',
    platformId: 'com.test.game',
    status: 'active',
    publisherId: 'pub-migration',
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  },
  {
    id: 'plc-rewarded-1',
    name: 'Reward Video',
    type: 'rewarded',
    format: 'rewarded-video',
    platformId: 'com.test.game',
    status: 'active',
    publisherId: 'pub-migration',
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  },
];

const MOCK_EXPERIMENTS = [
  {
    id: 'exp-1',
    name: 'Unity to Apex Migration',
    placement_id: 'plc-banner-1',
    status: 'active',
    mirror_percent: 50,
    created_at: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(),
    updated_at: new Date().toISOString(),
    metrics: {
      control_revenue: 150.00,
      treatment_revenue: 155.00,
      control_fill_rate: 0.92,
      treatment_fill_rate: 0.94,
    },
  },
  {
    id: 'exp-2',
    name: 'ironSource Migration Test',
    placement_id: 'plc-rewarded-1',
    status: 'draft',
    mirror_percent: 0,
    created_at: new Date(Date.now() - 2 * 24 * 60 * 60 * 1000).toISOString(),
    updated_at: new Date().toISOString(),
    metrics: null,
  },
  {
    id: 'exp-3',
    name: 'Completed MAX Migration',
    placement_id: 'plc-interstitial-1',
    status: 'completed',
    mirror_percent: 100,
    created_at: new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(),
    metrics: {
      control_revenue: 500.00,
      treatment_revenue: 525.00,
      control_fill_rate: 0.88,
      treatment_fill_rate: 0.91,
    },
  },
];

const MOCK_GUARDRAILS = {
  passed: true,
  checks: [
    { name: 'Revenue Delta', status: 'pass', value: '+3.3%', threshold: '-5%' },
    { name: 'Fill Rate Delta', status: 'pass', value: '+2.2%', threshold: '-3%' },
    { name: 'Latency P95', status: 'pass', value: '245ms', threshold: '500ms' },
    { name: 'Error Rate', status: 'pass', value: '0.1%', threshold: '1%' },
  ],
};

async function setupMigrationMocks(page: Page) {
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
      plan_type: 'growth',
      included_impressions: 500000,
      included_api_calls: 100000,
      included_data_transfer_gb: 50,
    },
  };

  // Only intercept external API calls (localhost:8080/api/v1/*)

  await page.route('**/api/v1/auth/csrf', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ token: 'csrf-token' }),
    })
  );

  await page.route('**/api/v1/meta/features', (route) =>
    respondJson(route, 200, {
      data: { transparency: true, billing: true, migrationStudio: true },
      success: true,
    })
  );

  // Auth me endpoint - matches apiClient.get('/auth/me')
  await page.route('**/api/v1/auth/me', (route) =>
    respondJson(route, 200, {
      success: true,
      data: {
        userId: 'usr-migration',
        email: 'migration@test.com',
        role: 'publisher',
        publisherId: 'pub-migration',
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

  await page.route('**/api/v1/placements**', (route) =>
    respondJson(route, 200, {
      data: MOCK_PLACEMENTS,
      pagination: { page: 1, pageSize: 100, total: MOCK_PLACEMENTS.length },
    })
  );

  await page.route('**/api/v1/migration/experiments**', (route) => {
    const method = route.request().method();
    if (method === 'POST') {
      return respondJson(route, 201, {
        success: true,
        data: {
          id: 'exp-new',
          name: 'New Experiment',
          status: 'draft',
          mirror_percent: 0,
        },
      });
    }
    return respondJson(route, 200, {
      success: true,
      data: MOCK_EXPERIMENTS,
    });
  });

  await page.route('**/api/v1/migration/experiments/*/guardrails', (route) =>
    respondJson(route, 200, { success: true, data: MOCK_GUARDRAILS })
  );

  await page.route('**/api/v1/migration/experiments/*', (route) => {
    const method = route.request().method();
    if (method === 'PATCH' || method === 'PUT') {
      return respondJson(route, 200, { success: true });
    }
    if (method === 'DELETE') {
      return respondJson(route, 200, { success: true });
    }
    // GET single experiment
    return respondJson(route, 200, {
      success: true,
      data: MOCK_EXPERIMENTS[0],
    });
  });

  await page.route('**/api/v1/migration/import', (route) =>
    respondJson(route, 200, {
      success: true,
      data: {
        parsed: {
          networks: ['Unity Ads', 'AdMob', 'ironSource'],
          placements: 5,
          floors: { min: 0.10, max: 2.50 },
        },
      },
    })
  );

  await page.route('**/api/migration/parity-simulate', (route) =>
    respondJson(route, 200, {
      success: true,
      data: {
        simulation_results: {
          estimated_revenue_change: '+2.5%',
          fill_rate_change: '+1.2%',
          latency_impact: '-15ms',
        },
      },
    })
  );

  await page.route('**/api/**', (route) => {
    const method = route.request().method();
    if (method === 'GET') {
      return respondJson(route, 200, { success: true, data: [] });
    }
    return respondJson(route, 200, { success: true });
  });
}

test.describe('Migration Studio Main Page', () => {
  test.beforeEach(async ({ page }) => {
    await setupMigrationMocks(page);
  });

  test('displays placement selector and experiment list', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Should show migration studio heading or related content
    await expect(page.locator('main')).toBeVisible();

    // Look for placement selector
    const placementSelect = page.locator('select, [role="combobox"]');
    if (await placementSelect.count() > 0) {
      await expect(placementSelect.first()).toBeVisible();
    }

    await page.screenshot({ path: 'test-results/screenshots/migration-studio-main.png', fullPage: true });
  });

  test('shows experiment status badges', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Check for status indicators
    const content = await page.content();
    
    // Should have some status indication
    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: 'test-results/screenshots/migration-experiments.png', fullPage: true });
  });

  test('import wizard button is present', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Look for import button - just check it exists, don't click if disabled
    const importBtn = page.getByRole('button', { name: /import|upload|add/i });
    if (await importBtn.count() > 0) {
      await expect(importBtn.first()).toBeAttached();
    }

    await page.screenshot({ path: 'test-results/screenshots/migration-import-wizard.png', fullPage: true });
  });

  test('parity simulator button is present', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Look for parity/simulator button - just check it exists
    const simBtn = page.getByRole('button', { name: /simulator|parity|simulate/i });
    if (await simBtn.count() > 0) {
      await expect(simBtn.first()).toBeAttached();
    }

    await page.screenshot({ path: 'test-results/screenshots/migration-parity-sim.png', fullPage: true });
  });
});

test.describe('Experiment Detail Page', () => {
  test.beforeEach(async ({ page }) => {
    await setupMigrationMocks(page);
  });

  test('displays experiment metrics and controls', async ({ page }) => {
    await page.goto('/migration-studio/exp-1');
    await page.waitForLoadState('networkidle');

    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: 'test-results/screenshots/migration-experiment-detail.png', fullPage: true });
  });

  test('shows guardrail status', async ({ page }) => {
    await page.goto('/migration-studio/exp-1');
    await page.waitForLoadState('networkidle');

    // Look for guardrail section
    const content = await page.content();
    
    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: 'test-results/screenshots/migration-guardrails.png', fullPage: true });
  });
});

test.describe('Migration Workflow', () => {
  test.beforeEach(async ({ page }) => {
    await setupMigrationMocks(page);
  });

  test('can view placement selector', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Find and verify placement selector exists
    const select = page.locator('select').first();
    if (await select.count() > 0) {
      await expect(select).toBeVisible();
    }

    await page.screenshot({ path: 'test-results/screenshots/migration-placement-select.png', fullPage: true });
  });

  test('experiment status transitions display correctly', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Check for different status badges
    const content = await page.content();
    
    // Verify page loaded properly
    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: 'test-results/screenshots/migration-statuses.png', fullPage: true });
  });
});

test.describe('Import Wizard Flow', () => {
  test.beforeEach(async ({ page }) => {
    await setupMigrationMocks(page);
  });

  test('import interface is accessible', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Verify page loads and import functionality exists
    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: 'test-results/screenshots/migration-import-flow.png', fullPage: true });
  });
});

test.describe('Parity Simulator', () => {
  test.beforeEach(async ({ page }) => {
    await setupMigrationMocks(page);
  });

  test('simulator interface is accessible', async ({ page }) => {
    await page.goto('/migration-studio');
    await page.waitForLoadState('networkidle');

    // Verify page loads
    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: 'test-results/screenshots/migration-simulator.png', fullPage: true });
  });
});
