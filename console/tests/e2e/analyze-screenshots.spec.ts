import { test, expect, Page, Route } from '@playwright/test';

type TierName = 'starter' | 'growth' | 'scale' | 'enterprise';

const respondJson = (route: Route, status: number, body: unknown) =>
  route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });

async function setupMocks(page: Page, tier: TierName = 'scale') {
  // Catch-all first (checked last due to LIFO)
  await page.route('**/api/v1/**', (route) => {
    const url = route.request().url();
    console.log('[Mock] Catch-all hit:', url);
    respondJson(route, 200, { success: true, data: {} });
  });

  await page.route('**/api/v1/auth/csrf', route => respondJson(route, 200, { token: 'csrf' }));
  
  await page.route('**/api/v1/auth/me', route => respondJson(route, 200, {
    success: true,
    data: { userId: 'usr-1', email: 'test@test.com', role: 'admin', publisherId: 'pub-1' }
  }));
  
  await page.route('**/api/auth/session', route => respondJson(route, 200, {
    user: { id: 'usr-1', email: 'test@test.com', role: 'admin', publisherId: 'pub-1' },
    expires: new Date(Date.now() + 86400000).toISOString()
  }));
  
  await page.route('**/api/v1/meta/features', route => respondJson(route, 200, {
    success: true, data: { transparency: true, billing: true, migrationStudio: true }
  }));
  
  await page.route('**/api/v1/billing/usage**', route => respondJson(route, 200, {
    current_period: { start: '2025-01-01', end: '2025-01-31', impressions: 100000, api_calls: 5000, data_transfer_gb: 10 },
    overages: { impressions: { amount: 0, cost: 0 }, api_calls: { amount: 0, cost: 0 }, data_transfer: { amount: 0, cost: 0 }, total_overage_cost: 0 },
    subscription: { plan_type: tier, included_impressions: 500000, included_api_calls: 100000, included_data_transfer_gb: 50 }
  }));

  // Adapters - returns Adapter[] directly (axios unwraps it from .data)
  await page.route('**/api/v1/adapters', route => respondJson(route, 200, [
    { id: 'adp-1', name: 'Google AdMob', network: 'admob', status: 'active', placementId: 'plc-1', bidFloor: 1.5, timeout: 300, lastSync: new Date().toISOString() },
    { id: 'adp-2', name: 'AppLovin', network: 'applovin', status: 'active', placementId: 'plc-2', bidFloor: 2.0, timeout: 250, lastSync: new Date().toISOString() },
  ]));

  // Placements - returns array directly
  await page.route('**/api/v1/placements', route => respondJson(route, 200, [
    { id: 'plc-1', name: 'Banner 320x50', type: 'banner', status: 'active', impressions: 100000, revenue: 500 },
    { id: 'plc-2', name: 'Interstitial', type: 'interstitial', status: 'active', impressions: 50000, revenue: 750 },
  ]));

  // Revenue timeseries - returns RevenueData[] directly
  await page.route('**/api/v1/revenue/timeseries**', route => respondJson(route, 200, [
    { date: '2025-01-01', revenue: 100, impressions: 10000, fillRate: 0.85, ecpm: 10.0 },
    { date: '2025-01-02', revenue: 120, impressions: 12000, fillRate: 0.87, ecpm: 10.0 },
    { date: '2025-01-03', revenue: 110, impressions: 11000, fillRate: 0.86, ecpm: 10.0 },
  ]));

  // Revenue summary
  await page.route('**/api/v1/revenue/summary**', route => respondJson(route, 200, {
    totalRevenue: 5000, totalImpressions: 100000, fillRate: 0.85, ecpm: 5.0, currency: 'USD'
  }));

  // Transparency auctions - returns { page, limit, count, data }
  await page.route('**/api/v1/transparency/auctions**', route => respondJson(route, 200, {
    page: 1,
    limit: 25,
    count: 2,
    data: [
      {
        auction_id: 'auc-001', timestamp: new Date().toISOString(), publisher_id: 'pub-1',
        app_or_site_id: 'app-1', placement_id: 'plc-1', surface_type: 'mobile_app',
        device_context: { os: 'iOS', geo: 'US', att: '3', tc_string_sha256: 'abc123' },
        candidates: [{ source: 'admob', bid_ecpm: 2.50, currency: 'USD', response_time_ms: 45, status: 'bid', metadata_hash: 'hash1' }],
        winner: { source: 'admob', bid_ecpm: 2.50, gross_price: 2.50, currency: 'USD', reason: 'highest_bid' },
        fees: { aletheia_fee_bp: 500, effective_publisher_share: 0.95 },
        integrity: { signature: 'sig123', algo: 'ed25519', key_id: 'key-1' }
      }
    ]
  }));

  await page.route('**/api/v1/fraud/**', route => respondJson(route, 200, {
    fraudRate: 0.01, totalEvents: 1000, flaggedEvents: 10, blockedEvents: 5, estimatedSavings: 100
  }));

  await page.route('**/api/v1/payouts/**', route => respondJson(route, 200, null));
  
  await page.route('**/api/v1/settings/**', route => respondJson(route, 200, {
    enabled: true, warningThreshold: 0.02, criticalThreshold: 0.05
  }));

  // Admin health endpoint
  await page.route('**/api/v1/admin/health**', route => respondJson(route, 200, {
    status: 'healthy',
    services: { database: 'ok', redis: 'ok', queue: 'ok' },
    uptime: 86400,
    version: '1.0.0'
  }));
}

const PAGES_TO_CHECK = [
  '/dashboard',
  '/placements', 
  '/adapters',
  '/analytics',
  '/payouts',
  '/settings',
  '/billing/usage',
  '/migration-studio',
  '/transparency/auctions',
  '/recon/overview',
  '/fraud',
  '/admin/health',
];

test.describe('Screenshot Content Analysis', () => {
  for (const pagePath of PAGES_TO_CHECK) {
    test(`analyze ${pagePath} for errors`, async ({ page }) => {
      const consoleErrors: string[] = [];
      const consoleWarnings: string[] = [];
      
      page.on('console', msg => {
        if (msg.type() === 'error') consoleErrors.push(msg.text());
        if (msg.type() === 'warning') consoleWarnings.push(msg.text());
      });

      await setupMocks(page);
      
      await page.goto(pagePath, { waitUntil: 'networkidle', timeout: 30000 });
      await page.waitForTimeout(1000);
      
      const bodyText = await page.textContent('body') || '';
      
      // Check for error indicators
      const errorPatterns = [
        { pattern: '403', name: '403 error' },
        { pattern: 'Forbidden', name: 'Forbidden' },
        { pattern: 'Cannot read properties', name: 'Cannot read properties' },
        { pattern: 'undefined', name: 'undefined' },
        { pattern: 'Error:', name: 'Error:' },
        { pattern: 'Something went wrong', name: 'Something went wrong' },
        { pattern: 'Network Error', name: 'Network Error' },
        { pattern: 'Failed to fetch', name: 'Failed to fetch' },
      ];
      
      const foundErrors: string[] = [];
      for (const { pattern, name } of errorPatterns) {
        if (bodyText.includes(pattern)) {
          foundErrors.push(name);
        }
      }
      
      // Report findings
      console.log(`\n=== ${pagePath} ===`);
      console.log(`Body length: ${bodyText.length} chars`);
      console.log(`Console errors: ${consoleErrors.length}`);
      if (consoleErrors.length > 0) {
        consoleErrors.forEach(e => console.log(`  ERROR: ${e.substring(0, 200)}`));
      }
      if (foundErrors.length > 0) {
        console.log(`Found in page text: ${foundErrors.join(', ')}`);
      }
      
      // Take a screenshot for manual review
      const safeName = pagePath.replace(/\//g, '-').replace(/^-/, '');
      await page.screenshot({ path: `test-results/analysis-${safeName}.png` });
      
      // Fail if there are critical errors (but not 'undefined' which might be in legitimate content)
      const criticalErrors = foundErrors.filter(e => 
        e === '403 error' || e === 'Forbidden' || e === 'Cannot read properties'
      );
      
      expect(criticalErrors, `Critical errors found: ${criticalErrors.join(', ')}`).toHaveLength(0);
      expect(consoleErrors.filter(e => 
        e.includes('Cannot read') || e.includes('TypeError') || e.includes('is not a function')
      ), 'JS errors in console').toHaveLength(0);
    });
  }
});
