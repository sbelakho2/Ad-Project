import { test, expect, Page, Route, devices } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

/**
 * Comprehensive Screenshot Generation Suite
 * Captures all console views across all device types
 */

type TierName = 'starter' | 'growth' | 'scale' | 'enterprise';

const SCREENSHOT_DIR = 'test-results/screenshots';

// Device configurations
const DEVICES = {
  desktop: { viewport: { width: 1920, height: 1080 }, name: 'desktop' },
  laptop: { viewport: { width: 1280, height: 720 }, name: 'laptop' },
  tablet: { viewport: { width: 768, height: 1024 }, name: 'tablet' },
  mobile: { viewport: { width: 375, height: 812 }, name: 'mobile' },
};

// All console routes to capture
const CONSOLE_ROUTES = [
  { path: '/dashboard', name: 'dashboard', minTier: 'starter' as TierName },
  { path: '/placements', name: 'placements', minTier: 'starter' as TierName },
  { path: '/adapters', name: 'adapters', minTier: 'starter' as TierName },
  { path: '/analytics', name: 'analytics', minTier: 'starter' as TierName },
  { path: '/payouts', name: 'payouts', minTier: 'starter' as TierName },
  { path: '/settings', name: 'settings', minTier: 'starter' as TierName },
  { path: '/billing/usage', name: 'billing-usage', minTier: 'starter' as TierName },
  { path: '/billing/settings', name: 'billing-settings', minTier: 'starter' as TierName },
  { path: '/migration-studio', name: 'migration-studio', minTier: 'starter' as TierName },
  { path: '/transparency/auctions', name: 'transparency-auctions', minTier: 'growth' as TierName },
  { path: '/recon/overview', name: 'vra-overview', minTier: 'growth' as TierName },
  { path: '/recon/deltas', name: 'vra-deltas', minTier: 'growth' as TierName },
  { path: '/fraud', name: 'fraud', minTier: 'growth' as TierName },
  { path: '/supply-chain', name: 'supply-chain', minTier: 'starter' as TierName },
  { path: '/tools/supply-chain-status', name: 'tools-supply-chain', minTier: 'starter' as TierName },
  { path: '/tools/app-ads-inspector', name: 'tools-app-ads', minTier: 'starter' as TierName },
  { path: '/tools/mediation-debugger', name: 'tools-debugger', minTier: 'starter' as TierName },
  { path: '/admin/health', name: 'admin-health', minTier: 'starter' as TierName, adminOnly: true },
];

const respondJson = (route: Route, status: number, body: unknown) =>
  route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });

async function setupComprehensiveMocks(page: Page, tier: TierName = 'scale') {
  // IMPORTANT: Playwright routes are matched in LIFO order (last registered = first checked)
  // So we register the catch-all FIRST, then specific routes AFTER so they take precedence

  // Catch-all for unhandled API routes (registered first = checked last)
  await page.route('**/api/v1/**', (route) => {
    const url = route.request().url();
    console.log('[Mock] Unhandled API call:', url);
    respondJson(route, 200, { success: true, data: {} });
  });

  // CSRF token endpoint
  await page.route('**/api/v1/auth/csrf', (route) => {
    respondJson(route, 200, { token: 'mock-csrf-token' });
  });

  // Session mock - must match apiClient.get('/auth/me') format
  await page.route('**/api/v1/auth/me', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        userId: 'screenshot-user',
        email: 'screenshots@apexmediation.com',
        role: 'admin',
        publisherId: 'pub-screenshot-001',
      },
    });
  });

  // Feature flags - must match apiClient.get('/meta/features') format
  await page.route('**/api/v1/meta/features', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        transparency: true,
        billing: true,
        migrationStudio: true,
      },
    });
  });

  // Billing usage - must match UsageData interface from billing.ts
  await page.route('**/api/v1/billing/usage**', (route) => {
    respondJson(route, 200, {
      current_period: {
        start: new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString(),
        end: new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString(),
        impressions: 2500000,
        api_calls: 150000,
        data_transfer_gb: 45,
      },
      overages: {
        impressions: { amount: 50000, cost: 25 },
        api_calls: { amount: 10000, cost: 5 },
        data_transfer: { amount: 5, cost: 2.5 },
        total_overage_cost: 32.5,
      },
      subscription: {
        plan_type: tier,
        included_impressions: 5000000,
        included_api_calls: 200000,
        included_data_transfer_gb: 100,
      },
    });
  });

  // NextAuth session endpoint (used by next-auth/react useSession)
  await page.route('**/api/auth/session', (route) => {
    respondJson(route, 200, {
      user: {
        id: 'screenshot-user',
        email: 'screenshots@apexmediation.com',
        role: 'admin',
        publisherId: 'pub-screenshot-001',
      },
      expires: new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString(),
    });
  });

  // Revenue summary (used by dashboard)
  await page.route('**/api/v1/revenue/summary**', (route) => {
    respondJson(route, 200, {
      data: {
        totalRevenue: 34500.00,
        totalImpressions: 3450000,
        fillRate: 0.87,
        ecpm: 10.04,
        currency: 'USD',
      },
    });
  });

  // Revenue timeseries (used by analytics charts) - returns RevenueData[] directly
  await page.route('**/api/v1/revenue/timeseries**', (route) => {
    const days = Array.from({ length: 30 }, (_, i) => {
      const date = new Date(Date.now() - (29 - i) * 24 * 60 * 60 * 1000);
      return {
        date: date.toISOString().split('T')[0],
        revenue: 1000 + Math.floor(Math.random() * 500),
        impressions: 100000 + Math.floor(Math.random() * 50000),
        clicks: 500 + Math.floor(Math.random() * 200),
        fill_rate: 0.8 + Math.random() * 0.15,
        ecpm: 8 + Math.random() * 4,
      };
    });
    respondJson(route, 200, days);
  });

  // Revenue summary (used by dashboard) - returns AnalyticsSummary
  await page.route('**/api/v1/revenue/summary**', (route) => {
    respondJson(route, 200, {
      totalRevenue: 34500.00,
      totalImpressions: 3450000,
      totalClicks: 25000,
      averageFillRate: 0.87,
      averageEcpm: 10.04,
      currency: 'USD',
    });
  });

  // Dashboard stats
  await page.route('**/api/v1/dashboard/**', (route) => {
    respondJson(route, 200, {
      revenue: { today: 1250.50, yesterday: 1180.25, week: 8450.75, month: 34500.00 },
      impressions: { today: 125000, yesterday: 118000, week: 845000, month: 3450000 },
      fillRate: 0.87,
      ecpm: 10.04,
      topNetworks: [
        { name: 'AdMob', revenue: 12500, impressions: 1200000 },
        { name: 'Unity Ads', revenue: 8500, impressions: 850000 },
        { name: 'IronSource', revenue: 6500, impressions: 700000 },
      ],
    });
  });

  // Placements - returns Placement[] directly (axios unwraps from .data)
  await page.route('**/api/v1/placements**', (route) => {
    respondJson(route, 200, [
      { id: 'pl-1', name: 'Home Banner', format: 'banner', status: 'active', ad_unit_id: 'au-1', app_id: 'app-1' },
      { id: 'pl-2', name: 'Interstitial Level Complete', format: 'interstitial', status: 'active', ad_unit_id: 'au-2', app_id: 'app-1' },
      { id: 'pl-3', name: 'Rewarded Video', format: 'rewarded', status: 'active', ad_unit_id: 'au-3', app_id: 'app-1' },
      { id: 'pl-4', name: 'Native Feed', format: 'native', status: 'paused', ad_unit_id: 'au-4', app_id: 'app-1' },
    ]);
  });

  // Adapters - returns Adapter[] directly (axios unwraps from .data)
  await page.route('**/api/v1/adapters**', (route) => {
    respondJson(route, 200, [
      { id: 'adp-1', name: 'AdMob', network: 'admob', status: 'active', credentials_valid: true, last_sync_at: new Date().toISOString() },
      { id: 'adp-2', name: 'Unity Ads', network: 'unity', status: 'active', credentials_valid: true, last_sync_at: new Date().toISOString() },
      { id: 'adp-3', name: 'IronSource', network: 'ironsource', status: 'active', credentials_valid: true, last_sync_at: new Date(Date.now() - 3600000).toISOString() },
      { id: 'adp-4', name: 'AppLovin MAX', network: 'applovin', status: 'inactive', credentials_valid: false, last_sync_at: null },
      { id: 'adp-5', name: 'Vungle', network: 'vungle', status: 'active', credentials_valid: true, last_sync_at: new Date().toISOString() },
    ]);
  });

  // Analytics - returns { data: [...] }
  await page.route('**/api/v1/analytics/**', (route) => {
    const days = Array.from({ length: 30 }, (_, i) => {
      const date = new Date(Date.now() - (29 - i) * 24 * 60 * 60 * 1000);
      return {
        date: date.toISOString().split('T')[0],
        impressions: 100000 + Math.floor(Math.random() * 50000),
        revenue: 1000 + Math.floor(Math.random() * 500),
        fillRate: 0.8 + Math.random() * 0.15,
        ecpm: 8 + Math.random() * 4,
      };
    });
    respondJson(route, 200, { data: days });
  });

  // Transparency auctions - returns { page, limit, count, data: TransparencyAuction[] }
  await page.route('**/api/v1/transparency/auctions**', (route) => {
    respondJson(route, 200, {
      page: 1,
      limit: 25,
      count: 3,
      data: [
        {
          auction_id: 'auc-1', timestamp: new Date().toISOString(), publisher_id: 'pub-screenshot-001',
          app_or_site_id: 'app-1', placement_id: 'pl-1', surface_type: 'mobile_app',
          device_context: { os: 'iOS', geo: 'US', att: '3', tc_string_sha256: 'abc123' },
          candidates: [
            { source: 'admob', bid_ecpm: 12.50, currency: 'USD', response_time_ms: 45, status: 'bid', metadata_hash: 'hash1' },
            { source: 'unity', bid_ecpm: 10.25, currency: 'USD', response_time_ms: 52, status: 'bid', metadata_hash: 'hash2' },
          ],
          winner: { source: 'admob', bid_ecpm: 12.50, gross_price: 12.50, currency: 'USD', reason: 'highest_bid' },
          fees: { aletheia_fee_bp: 500, effective_publisher_share: 0.95 },
          integrity: { signature: 'sig-1', algo: 'ed25519', key_id: 'key-1' }
        },
        {
          auction_id: 'auc-2', timestamp: new Date(Date.now() - 60000).toISOString(), publisher_id: 'pub-screenshot-001',
          app_or_site_id: 'app-1', placement_id: 'pl-2', surface_type: 'mobile_app',
          device_context: { os: 'Android', geo: 'UK', att: '0', tc_string_sha256: 'def456' },
          candidates: [
            { source: 'unity', bid_ecpm: 8.75, currency: 'USD', response_time_ms: 38, status: 'bid', metadata_hash: 'hash3' },
          ],
          winner: { source: 'unity', bid_ecpm: 8.75, gross_price: 8.75, currency: 'USD', reason: 'highest_bid' },
          fees: { aletheia_fee_bp: 500, effective_publisher_share: 0.95 },
          integrity: { signature: 'sig-2', algo: 'ed25519', key_id: 'key-1' }
        },
        {
          auction_id: 'auc-3', timestamp: new Date(Date.now() - 120000).toISOString(), publisher_id: 'pub-screenshot-001',
          app_or_site_id: 'app-2', placement_id: 'pl-3', surface_type: 'web',
          device_context: { os: 'Windows', geo: 'DE', att: '0', tc_string_sha256: 'ghi789' },
          candidates: [
            { source: 'ironsource', bid_ecpm: 15.25, currency: 'USD', response_time_ms: 62, status: 'bid', metadata_hash: 'hash4' },
            { source: 'admob', bid_ecpm: 14.00, currency: 'USD', response_time_ms: 48, status: 'bid', metadata_hash: 'hash5' },
          ],
          winner: { source: 'ironsource', bid_ecpm: 15.25, gross_price: 15.25, currency: 'USD', reason: 'highest_bid' },
          fees: { aletheia_fee_bp: 500, effective_publisher_share: 0.95 },
          integrity: { signature: 'sig-3', algo: 'ed25519', key_id: 'key-1' }
        },
      ],
    });
  });

  // VRA Reconciliation
  await page.route('**/api/v1/recon/overview', (route) => {
    respondJson(route, 200, {
      networks: [
        { network: 'AdMob', expected: 15000, paid: 14850, variance: -150, variancePercent: -1.0, status: 'reconciled' },
        { network: 'Unity Ads', expected: 9500, paid: 9500, variance: 0, variancePercent: 0, status: 'matched' },
        { network: 'IronSource', expected: 7200, paid: 6800, variance: -400, variancePercent: -5.5, status: 'disputed' },
      ],
      summary: { totalExpected: 31700, totalPaid: 31150, totalVariance: -550 },
    });
  });

  await page.route('**/api/v1/recon/deltas', (route) => {
    respondJson(route, 200, {
      items: [
        { id: 'd-1', network: 'IronSource', kind: 'underpayment', amount: 250, currency: 'USD', reasonCode: 'FILL_DISCREPANCY', windowStart: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(), windowEnd: new Date().toISOString(), evidenceId: 'ev-001', confidence: 0.92 },
        { id: 'd-2', network: 'AdMob', kind: 'timing', amount: 150, currency: 'USD', reasonCode: 'LATE_ATTRIBUTION', windowStart: new Date(Date.now() - 14 * 24 * 60 * 60 * 1000).toISOString(), windowEnd: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(), evidenceId: 'ev-002', confidence: 0.85 },
        { id: 'd-3', network: 'Unity Ads', kind: 'overpayment', amount: 50, currency: 'USD', reasonCode: 'DUPLICATE_EVENT', windowStart: new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString(), windowEnd: new Date().toISOString(), evidenceId: 'ev-003', confidence: 0.98 },
      ],
      page: 1,
      pageSize: 50,
      total: 3,
    });
  });

  // Fraud detection - handle various paths
  await page.route('**/api/v1/fraud/**', (route) => {
    const url = route.request().url();
    if (url.includes('/stats/')) {
      return respondJson(route, 200, {
        fraudRate: 0.015,
        totalEvents: 5000,
        flaggedEvents: 75,
        blockedEvents: 50,
        estimatedSavings: 1250,
      });
    }
    if (url.includes('/alerts/')) {
      return respondJson(route, 200, {
        alerts: [
          { id: 'f-1', type: 'click_injection', severity: 'high', timestamp: new Date().toISOString(), source: 'AdNetwork-X' },
          { id: 'f-2', type: 'sdk_spoofing', severity: 'critical', timestamp: new Date(Date.now() - 3600000).toISOString(), source: 'Unknown' },
        ],
      });
    }
    respondJson(route, 200, {
      summary: { totalEvents: 5000, flaggedEvents: 125, blockRate: 0.025, estimatedSavings: 1250 },
      recentFlags: [
        { id: 'f-1', type: 'click_injection', severity: 'high', timestamp: new Date().toISOString(), source: 'AdNetwork-X' },
        { id: 'f-2', type: 'sdk_spoofing', severity: 'critical', timestamp: new Date(Date.now() - 3600000).toISOString(), source: 'Unknown' },
        { id: 'f-3', type: 'device_farm', severity: 'medium', timestamp: new Date(Date.now() - 7200000).toISOString(), source: 'Region-CN' },
      ],
    });
  });

  // Migration Studio - uses /migration/experiments endpoint
  await page.route('**/api/v1/migration/**', (route) => {
    const url = route.request().url();
    if (url.includes('/experiments')) {
      return respondJson(route, 200, {
        success: true,
        data: [
          { id: 'exp-1', name: 'AdMob to Apex Banner', status: 'active', mirror_percent: 65, created_at: new Date(Date.now() - 14 * 24 * 60 * 60 * 1000).toISOString() },
          { id: 'exp-2', name: 'Unity Interstitial Migration', status: 'completed', mirror_percent: 100, created_at: new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString() },
          { id: 'exp-3', name: 'IronSource Rewarded Test', status: 'paused', mirror_percent: 35, created_at: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString() },
        ],
      });
    }
    respondJson(route, 200, { success: true, data: [] });
  });

  // Payouts - must match before the catch-all
  await page.route('**/api/v1/payouts/**', (route) => {
    const url = route.request().url();
    if (url.includes('/upcoming')) {
      return respondJson(route, 200, {
        id: 'pay-upcoming',
        amount: 6100.00,
        currency: 'USD',
        status: 'scheduled',
        scheduledFor: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString(),
        method: 'wire',
      });
    }
    if (url.includes('/history')) {
      return respondJson(route, 200, {
        items: [
          { id: 'pay-1', amount: 5420.50, currency: 'USD', status: 'completed', scheduledFor: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(), method: 'wire' },
          { id: 'pay-2', amount: 4850.25, currency: 'USD', status: 'completed', scheduledFor: new Date(Date.now() - 37 * 24 * 60 * 60 * 1000).toISOString(), method: 'wire' },
        ],
        total: 2,
        page: 1,
        pageSize: 25,
      });
    }
    respondJson(route, 200, { success: true });
  });

  await page.route('**/api/v1/payouts', (route) => {
    respondJson(route, 200, {
      items: [
        { id: 'pay-1', amount: 5420.50, currency: 'USD', status: 'completed', scheduledFor: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(), method: 'wire' },
        { id: 'pay-2', amount: 4850.25, currency: 'USD', status: 'completed', scheduledFor: new Date(Date.now() - 37 * 24 * 60 * 60 * 1000).toISOString(), method: 'wire' },
        { id: 'pay-3', amount: 6100.00, currency: 'USD', status: 'scheduled', scheduledFor: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString(), method: 'wire' },
      ],
      nextPayout: { amount: 6100.00, date: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString() },
    });
  });

  // Settings - multiple endpoints
  await page.route('**/api/v1/settings/**', (route) => {
    const url = route.request().url();
    if (url.includes('/fraud')) {
      return respondJson(route, 200, {
        enabled: true,
        warningThreshold: 0.02,
        criticalThreshold: 0.05,
        autoBlock: true,
      });
    }
    if (url.includes('/payout')) {
      return respondJson(route, 200, {
        threshold: 100,
        method: 'wire',
        currency: 'USD',
        schedule: 'monthly',
        accountName: 'Demo Publisher',
        accountReference: '****1234',
        autoPayout: true,
      });
    }
    respondJson(route, 200, {
      profile: { name: 'Demo Publisher', email: 'demo@apexmediation.com', company: 'Apex Games Inc.' },
      notifications: { email: true, slack: false, webhook: true },
    });
  });

  // Supply chain
  await page.route('**/api/v1/supply-chain/**', (route) => {
    respondJson(route, 200, {
      sellers: [
        { id: 'sel-1', domain: 'apexmediation.com', type: 'DIRECT', status: 'verified' },
        { id: 'sel-2', domain: 'partner-exchange.com', type: 'RESELLER', status: 'verified' },
      ],
      status: 'healthy',
      lastCheck: new Date().toISOString(),
    });
  });

  // Admin health
  await page.route('**/api/v1/admin/health', (route) => {
    respondJson(route, 200, {
      services: [
        { name: 'API Gateway', status: 'healthy', latency: 12 },
        { name: 'Database', status: 'healthy', latency: 5 },
        { name: 'Redis Cache', status: 'healthy', latency: 2 },
        { name: 'ML Pipeline', status: 'warning', latency: 250 },
        { name: 'Kafka', status: 'healthy', latency: 8 },
      ],
      uptime: 99.99,
      lastIncident: new Date(Date.now() - 45 * 24 * 60 * 60 * 1000).toISOString(),
    });
  });

  // Tools endpoints
  await page.route('**/api/v1/tools/**', (route) => {
    respondJson(route, 200, { status: 'ok', data: {} });
  });
}

async function waitForFonts(page: Page) {
  // Wait for document.fonts.ready to ensure all fonts are loaded
  await page.evaluate(async () => {
    await document.fonts.ready;
    // Additional wait for font rendering to complete
    await new Promise(resolve => setTimeout(resolve, 100));
  });
}

async function captureScreenshot(page: Page, routePath: string, routeName: string, deviceName: string) {
  const screenshotPath = path.join(SCREENSHOT_DIR, `${routeName}-${deviceName}.png`);
  
  try {
    await page.goto(routePath, { waitUntil: 'networkidle', timeout: 30000 });
    
    // Wait for fonts to be fully loaded and rendered
    await waitForFonts(page);
    
    // Allow animations to settle after fonts load
    await page.waitForTimeout(300);
    
    await page.screenshot({
      path: screenshotPath,
      fullPage: false,
    });
    
    console.log(`✓ Captured: ${screenshotPath}`);
    return true;
  } catch (error) {
    console.error(`✗ Failed: ${routeName}-${deviceName}: ${error}`);
    return false;
  }
}

test.describe('Screenshot Generation Suite', () => {
  test.beforeAll(async () => {
    // Ensure screenshot directory exists
    const fullPath = path.resolve(SCREENSHOT_DIR);
    if (!fs.existsSync(fullPath)) {
      fs.mkdirSync(fullPath, { recursive: true });
    }
  });

  // Desktop screenshots (1920x1080)
  test.describe('Desktop Views (1920x1080)', () => {
    test.use({ viewport: DEVICES.desktop.viewport });

    for (const route of CONSOLE_ROUTES) {
      test(`capture ${route.name}`, async ({ page }) => {
        await setupComprehensiveMocks(page, 'scale');
        await captureScreenshot(page, route.path, route.name, 'desktop');
        await expect(page.locator('body')).toBeVisible();
      });
    }
  });

  // Laptop screenshots (1280x720)
  test.describe('Laptop Views (1280x720)', () => {
    test.use({ viewport: DEVICES.laptop.viewport });

    for (const route of CONSOLE_ROUTES) {
      test(`capture ${route.name}`, async ({ page }) => {
        await setupComprehensiveMocks(page, 'scale');
        await captureScreenshot(page, route.path, route.name, 'laptop');
        await expect(page.locator('body')).toBeVisible();
      });
    }
  });

  // Tablet screenshots (768x1024)
  test.describe('Tablet Views (768x1024)', () => {
    test.use({ viewport: DEVICES.tablet.viewport });

    for (const route of CONSOLE_ROUTES) {
      test(`capture ${route.name}`, async ({ page }) => {
        await setupComprehensiveMocks(page, 'scale');
        await captureScreenshot(page, route.path, route.name, 'tablet');
        await expect(page.locator('body')).toBeVisible();
      });
    }
  });

  // Mobile screenshots (375x812 - iPhone X)
  test.describe('Mobile Views (375x812)', () => {
    test.use({ viewport: DEVICES.mobile.viewport });

    for (const route of CONSOLE_ROUTES) {
      test(`capture ${route.name}`, async ({ page }) => {
        await setupComprehensiveMocks(page, 'scale');
        await captureScreenshot(page, route.path, route.name, 'mobile');
        await expect(page.locator('body')).toBeVisible();
      });
    }
  });

  // Login page (no auth needed)
  test.describe('Login Page', () => {
    for (const [deviceKey, device] of Object.entries(DEVICES)) {
      test(`capture login - ${deviceKey}`, async ({ page }) => {
        await page.setViewportSize(device.viewport);
        await page.goto('/login', { waitUntil: 'networkidle' });
        await page.screenshot({
          path: path.join(SCREENSHOT_DIR, `login-${device.name}.png`),
          fullPage: false,
        });
        await expect(page.locator('body')).toBeVisible();
      });
    }
  });

  // Tier comparison - show gated vs ungated views
  test.describe('Tier Gating Comparison', () => {
    const gatedRoutes = [
      { path: '/transparency/auctions', name: 'transparency' },
      { path: '/recon/overview', name: 'vra' },
      { path: '/fraud', name: 'fraud' },
    ];

    for (const route of gatedRoutes) {
      test(`capture ${route.name} - starter (gated)`, async ({ page }) => {
        await setupComprehensiveMocks(page, 'starter');
        await page.setViewportSize(DEVICES.desktop.viewport);
        await page.goto(route.path, { waitUntil: 'networkidle' });
        await page.waitForTimeout(500);
        await page.screenshot({
          path: path.join(SCREENSHOT_DIR, `${route.name}-starter-gated-desktop.png`),
          fullPage: false,
        });
        await expect(page.locator('body')).toBeVisible();
      });

      test(`capture ${route.name} - growth (full access)`, async ({ page }) => {
        await setupComprehensiveMocks(page, 'growth');
        await page.setViewportSize(DEVICES.desktop.viewport);
        await page.goto(route.path, { waitUntil: 'networkidle' });
        await page.waitForTimeout(500);
        await page.screenshot({
          path: path.join(SCREENSHOT_DIR, `${route.name}-growth-access-desktop.png`),
          fullPage: false,
        });
        await expect(page.locator('body')).toBeVisible();
      });
    }
  });
});
