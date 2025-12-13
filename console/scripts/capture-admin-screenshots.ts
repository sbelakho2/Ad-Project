import { chromium, type Page, type Route } from 'playwright';

const ADMIN_PAGES = [
  { url: '/admin/health', name: 'health-dashboard' },
  { url: '/admin/billing', name: 'billing-operations' },
  { url: '/admin/audit', name: 'billing-audit' },
  { url: '/admin/infrastructure', name: 'infrastructure-monitoring' },
  { url: '/admin/sales-automation', name: 'sales-automation' },
  { url: '/admin/value-multipliers', name: 'value-multipliers' },
];

const BASE_URL = 'https://console.apexmediation.ee';
const OUTPUT_DIR = '/Users/sabelakhoua/IdeaProjects/Ad-Project/evidence-2025-12-08/admin';

const respondJson = (route: Route, status: number, body: unknown) =>
  route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });

async function setupAuthMocks(page: Page) {
  // Mock CSRF token
  await page.route('**/api/v1/auth/csrf', (route) => {
    respondJson(route, 200, { token: 'mock-csrf-token' });
  });

  // Mock session endpoint for our API client
  await page.route('**/api/v1/auth/me', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        userId: 'operator-screenshot-user',
        email: 'operator@apexmediation.ee',
        role: 'operator', // Operator role, not customer admin
        publisherId: 'internal',
      },
    });
  });

  // Mock NextAuth session endpoint - returns operator email for whitelist check
  await page.route('**/api/auth/session', (route) => {
    respondJson(route, 200, {
      user: {
        id: 'operator-screenshot-user',
        email: 'operator@apexmediation.ee', // Must match NEXT_PUBLIC_OPERATOR_EMAILS
        role: 'operator',
      },
      expires: new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString(),
    });
  });

  // Mock feature flags
  await page.route('**/api/v1/meta/features', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        transparency: true,
        billing: true,
        adminDashboard: true,
        requireAdminGuard: false, // Disable guard for screenshots - allows bypass
      },
    });
  });

  // Mock health endpoint
  await page.route('**/api/v1/meta/info', (route) => {
    respondJson(route, 200, {
      name: 'apexmediation-backend',
      version: '1.5.2',
      environment: 'production',
    });
  });

  // Mock RED metrics
  await page.route('**/api/v1/admin/red', (route) => {
    respondJson(route, 200, {
      api_rps_5m: 125.4,
      api_error_rate_5m: 0.002,
      api_p95_latency_5m: 0.045,
      rtb_p95_latency_5m: 0.12,
      rtb_adapter_timeouts_rps_5m: 0.3,
      timestamp: new Date().toISOString(),
    });
  });

  // Mock billing reconciliation status
  await page.route('**/api/v1/billing/reconcile', (route) => {
    if (route.request().method() === 'GET') {
      respondJson(route, 200, {
        last_run: new Date(Date.now() - 3600000).toISOString(),
        status: 'success',
        discrepancies: 0,
      });
    } else {
      respondJson(route, 200, { success: true });
    }
  });

  // Mock audit logs
  await page.route('**/api/v1/admin/audit**', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        logs: [
          { id: '1', timestamp: new Date().toISOString(), actor: 'admin@apexmediation.ee', action: 'billing.reconcile', details: 'Manual reconciliation triggered' },
          { id: '2', timestamp: new Date(Date.now() - 3600000).toISOString(), actor: 'system', action: 'billing.invoice.sent', details: 'Invoice INV-2025-001 sent' },
          { id: '3', timestamp: new Date(Date.now() - 7200000).toISOString(), actor: 'admin@apexmediation.ee', action: 'config.multiplier.update', details: 'Updated USD multiplier to 1.02' },
        ],
        total: 3,
      },
    });
  });

  // Mock infrastructure/autopilot endpoints
  await page.route('**/health', (route) => {
    if (route.request().url().includes('autopilot') || route.request().url().includes('8090')) {
      respondJson(route, 200, {
        state: 'OK',
        checks: {
          database: { status: 'OK', latency_ms: 5 },
          redis: { status: 'OK', latency_ms: 2 },
          storage: { status: 'OK', available_gb: 450 },
        },
        timestamp: new Date().toISOString(),
      });
    } else {
      route.continue();
    }
  });

  // Mock autopilot status
  await page.route('**/status', (route) => {
    respondJson(route, 200, {
      state: 'OK',
      last_check: new Date().toISOString(),
      checks: { partition_manager: { status: 'OK' }, recon: { status: 'OK' } },
      metrics: { jobs_completed_24h: 48, avg_duration_ms: 1250 },
      uptime: 864000,
      version: '1.2.0',
    });
  });

  // Mock database metrics
  await page.route('**/database/metrics', (route) => {
    respondJson(route, 200, {
      connections: { active: 12, idle: 8, total: 20, max: 100, usage_pct: 20 },
      tables: [
        { schema: 'public', name: 'auctions', row_count: 15000000, size_bytes: 2500000000, dead_tuples: 1200 },
        { schema: 'public', name: 'bids', row_count: 45000000, size_bytes: 8500000000, dead_tuples: 3500 },
      ],
      partitions: [
        { table: 'auctions', partition_count: 30, oldest_partition: '2025-11-01', newest_partition: '2025-12-13' },
      ],
      performance: { cache_hit_ratio: 0.994, transactions_per_sec: 1250, tuples_returned_per_sec: 45000 },
    });
  });

  // Mock jobs
  await page.route('**/jobs**', (route) => {
    respondJson(route, 200, {
      jobs: [
        { id: 'job-1', job_type: 'partition_maintenance', status: 'completed', scheduled_at: new Date(Date.now() - 3600000).toISOString(), completed_at: new Date(Date.now() - 3590000).toISOString(), duration_ms: 10000 },
        { id: 'job-2', job_type: 'recon_daily', status: 'completed', scheduled_at: new Date(Date.now() - 7200000).toISOString(), completed_at: new Date(Date.now() - 7190000).toISOString(), duration_ms: 8500 },
      ],
    });
  });

  // Mock sales automation - must match SalesAutomationOverview type
  await page.route('**/api/v1/admin/sales-automation**', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        summary: {
          trialToPaidRate: 23.5,
          baselineTrialRate: 15,
          avgDealSizeUSD: 175,
          targetDealSizeUSD: 200,
          timeToConvertDays: 7.5,
          targetTimeToConvertDays: 5,
          activeTrials: 48,
          trialWeekChange: 5,
        },
        principles: [
          { key: 'reciprocity', label: 'Reciprocity', description: 'Free value before ask', conversionRate: 0.23, touches: 1250, revenueUSD: 45000, color: 'emerald' },
          { key: 'scarcity', label: 'Scarcity', description: 'Limited-time offers', conversionRate: 0.31, touches: 890, revenueUSD: 38000, color: 'orange' },
          { key: 'authority', label: 'Authority', description: 'Expert endorsements', conversionRate: 0.18, touches: 650, revenueUSD: 22000, color: 'blue' },
          { key: 'consensus', label: 'Social Proof', description: 'Customer testimonials', conversionRate: 0.35, touches: 2100, revenueUSD: 67000, color: 'cyan' },
        ],
        funnel: [
          { key: 'visitor', name: 'Website Visitor', count: 10000, conversionPercentage: 100, avgDays: 0, engagementScore: 0.1 },
          { key: 'signup', name: 'Signed Up', count: 850, conversionPercentage: 8.5, avgDays: 0.5, engagementScore: 0.3 },
          { key: 'trial', name: 'Started Trial', count: 120, conversionPercentage: 1.2, avgDays: 1, engagementScore: 0.5 },
          { key: 'activated', name: 'Activated', count: 72, conversionPercentage: 0.72, avgDays: 3, engagementScore: 0.7 },
          { key: 'paid', name: 'Paid Customer', count: 25, conversionPercentage: 0.25, avgDays: 7.5, engagementScore: 0.9 },
        ],
        journey: [
          { day: 0, title: 'Welcome Email', principle: 'Reciprocity', sent: 850, openRate: 0.65, clickRate: 0.22, impact: 'High' },
          { day: 1, title: 'Quick Start Guide', principle: 'Authority', sent: 720, openRate: 0.58, clickRate: 0.18, impact: 'High' },
          { day: 5, title: 'Case Study', principle: 'Social Proof', sent: 620, openRate: 0.52, clickRate: 0.25, impact: 'Very High' },
          { day: 7, title: 'Trial Ending', principle: 'Scarcity', sent: 580, openRate: 0.68, clickRate: 0.35, impact: 'Critical' },
        ],
        conversions: [],
        experiments: [
          { id: 'exp-1', testName: 'Welcome Email Subject', hypothesis: 'Personalized subject increases open rate', controlRate: 0.52, testRate: 0.65, confidence: 0.95, winner: 'test', recommendation: 'Roll out test variant' },
        ],
        quickActions: [
          { id: 'qa-1', title: 'Send Reminder', description: 'Send trial expiry reminder to all users' },
          { id: 'qa-2', title: 'Export Leads', description: 'Download leads CSV for CRM import' },
        ],
        insights: [
          'Trial-to-paid conversion is above industry baseline by 8 percentage points',
          'Social proof touchpoints show highest engagement (35% conversion rate)',
        ],
        updatedAt: new Date().toISOString(),
      },
    });
  });

  // Mock value multipliers
  await page.route('**/api/v1/admin/multipliers**', (route) => {
    respondJson(route, 200, {
      success: true,
      data: {
        multipliers: [
          { currency: 'USD', value: 1.0, updated_at: new Date().toISOString() },
          { currency: 'EUR', value: 1.08, updated_at: new Date().toISOString() },
          { currency: 'GBP', value: 1.26, updated_at: new Date().toISOString() },
        ],
        geo_multipliers: [
          { region: 'US', value: 1.0 },
          { region: 'EU', value: 0.95 },
          { region: 'APAC', value: 0.88 },
        ],
      },
    });
  });

  // Catch-all for unhandled API routes
  await page.route('**/api/**', (route) => {
    const url = route.request().url();
    console.log('[Mock] Unhandled API call:', url);
    respondJson(route, 200, { success: true, data: {} });
  });
}

// Set operator credentials in session storage for the new operator auth system
async function injectOperatorCredentials(page: Page) {
  // This must be done after navigating to the domain (even to a blank page on the domain)
  // The operator auth system checks sessionStorage for 'operator_secret' and 'operator_email'
  await page.evaluate(() => {
    sessionStorage.setItem('operator_secret', 'screenshot-operator-secret');
    sessionStorage.setItem('operator_email', 'operator@apexmediation.ee');
  });
  console.log('✓ Injected operator credentials into session storage\n');
}

async function captureScreenshots() {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();

  // Setup all API mocks before navigating
  await setupAuthMocks(page);

  console.log('Starting admin page screenshot capture...\n');
  console.log('Using operator authentication mocking for admin access\n');

  // First, capture the login page (no mocks needed for this)
  console.log('Capturing login page...');
  try {
    const loginPage = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 2 });
    const lp = await loginPage.newPage();
    await lp.goto(`${BASE_URL}/login`, { waitUntil: 'networkidle', timeout: 30000 });
    await lp.waitForTimeout(2000);
    await lp.screenshot({ path: `${OUTPUT_DIR}/00-login-page.png`, fullPage: true });
    console.log('✓ Captured login page\n');
    await loginPage.close();
  } catch (error) {
    console.log(`✗ Login page error: ${error}\n`);
  }

  // Capture each admin page with mocked auth
  for (const { url, name } of ADMIN_PAGES) {
    console.log(`Capturing ${name}...`);
    try {
      const fullUrl = `${BASE_URL}${url}`;
      
      // Navigate to the page first
      const response = await page.goto(fullUrl, { waitUntil: 'networkidle', timeout: 30000 });
      
      // Inject operator credentials (must be done after being on the domain)
      await injectOperatorCredentials(page);
      
      // Reload to apply operator credentials
      await page.reload({ waitUntil: 'networkidle', timeout: 30000 });
      
      const status = response?.status() || 'unknown';
      const finalUrl = page.url();
      
      // Wait for content to load and React to hydrate
      await page.waitForTimeout(3000);
      
      // Wait for main content to appear
      try {
        await page.waitForSelector('main, [role="main"], .admin-content, h1, h2', { timeout: 5000 });
      } catch {
        console.log('  ⚠ No main content selector found, continuing...');
      }
      
      // Get page content summary
      const bodyText = await page.locator('body').innerText();
      const textPreview = bodyText.substring(0, 300).replace(/\s+/g, ' ').trim();
      
      // Take screenshot
      await page.screenshot({ 
        path: `${OUTPUT_DIR}/${name}.png`, 
        fullPage: true 
      });
      
      // Get page title
      const title = await page.title();
      
      console.log(`  ✓ Status: ${status}`);
      console.log(`  ✓ Title: ${title}`);
      console.log(`  ✓ Final URL: ${finalUrl}`);
      console.log(`  ✓ Content: ${textPreview.substring(0, 100)}...`);
      console.log(`  ✓ Screenshot saved: ${name}.png\n`);
      
      // Check if redirected to login (indicates auth mock isn't working)
      if (finalUrl.includes('login') || finalUrl.includes('auth') || finalUrl.includes('403')) {
        console.log(`  ⚠ Page may require additional authentication mocking\n`);
      }
    } catch (error) {
      console.log(`  ✗ Error: ${error}\n`);
    }
  }

  await browser.close();
  console.log('Screenshot capture complete!');
  console.log(`Screenshots saved to: ${OUTPUT_DIR}`);
}

captureScreenshots().catch(console.error);
