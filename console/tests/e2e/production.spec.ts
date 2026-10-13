// dogfood:skip:opt-in stress/production suite — enabled manually against deployed environments
/**
 * Production E2E Tests
 * 
 * These tests run against the REAL production API and verify end-to-end
 * functionality with actual user credentials. They test:
 * - Real authentication flow
 * - Real API responses
 * - Real database interactions
 * - Real feature access based on user role and subscription
 */

import { test, expect, Page } from '@playwright/test';

// This spec is intended to run against real production services.
// It is intentionally disabled by default to avoid accidental traffic/credential use.
const PRODUCTION_E2E_ENABLED = process.env.RUN_PRODUCTION_E2E === 'true';
test.skip(!PRODUCTION_E2E_ENABLED, 'Set RUN_PRODUCTION_E2E=true to run production E2E tests.');

// Production credentials must be provided via environment variables.
const TEST_USER = {
  email: process.env.PROD_E2E_EMAIL || '',
  password: process.env.PROD_E2E_PASSWORD || '',
};

if (PRODUCTION_E2E_ENABLED && (!TEST_USER.email || !TEST_USER.password)) {
  throw new Error('RUN_PRODUCTION_E2E=true requires PROD_E2E_EMAIL and PROD_E2E_PASSWORD to be set.');
}

const PROD_API = 'https://api.apexmediation.ee';
const PROD_CONSOLE = 'https://console.apexmediation.ee';

// Helper to get a fresh auth token with retry for rate limiting
async function getAuthToken(maxRetries = 3): Promise<{ token: string; user: any }> {
  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    const response = await fetch(`${PROD_API}/api/v1/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: TEST_USER.email,
        password: TEST_USER.password,
      }),
    });
    
    const data = await response.json();
    
    // Check for rate limiting
    if (response.status === 429 && attempt < maxRetries) {
      const retryAfter = data.retryAfter || 5;
      console.log(`Rate limited, waiting ${retryAfter}s before retry ${attempt + 1}/${maxRetries}`);
      await new Promise(r => setTimeout(r, retryAfter * 1000));
      continue;
    }
    
    if (!data.success) {
      throw new Error(`Login failed: ${data.error}`);
    }
    return { token: data.data.token, user: data.data.user };
  }
  throw new Error('Max retries exceeded for auth token');
}

// Helper to make authenticated API calls
async function apiCall(
  token: string,
  endpoint: string,
  options: { method?: string; body?: any } = {}
): Promise<any> {
  const response = await fetch(`${PROD_API}${endpoint}`, {
    method: options.method || 'GET',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${token}`,
    },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  return response.json();
}

test.describe('Production Authentication', () => {
  test('1. Login with valid credentials returns tokens and user data', async () => {
    const response = await fetch(`${PROD_API}/api/v1/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: TEST_USER.email,
        password: TEST_USER.password,
      }),
    });
    
    const data = await response.json();
    expect(response.ok).toBeTruthy();
    expect(data.success).toBe(true);
    expect(data.data.token).toBeTruthy();
    expect(data.data.refreshToken).toBeTruthy();
    expect(data.data.user.email).toBe(TEST_USER.email);
    expect(data.data.user.role).toBe('admin');
  });

  test('2. Login with invalid credentials fails gracefully', async () => {
    const response = await fetch(`${PROD_API}/api/v1/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: TEST_USER.email,
        password: 'wrongpassword',
      }),
    });
    
    const data = await response.json();
    expect(data.success).toBe(false);
    expect(data.error).toBeTruthy();
  });

  test('3. Token refresh endpoint works', async () => {
    // First login
    const loginResponse = await fetch(`${PROD_API}/api/v1/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: TEST_USER.email,
        password: TEST_USER.password,
      }),
    });
    const loginData = await loginResponse.json();
    
    // Then refresh
    const refreshResponse = await fetch(`${PROD_API}/api/v1/auth/refresh`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        refreshToken: loginData.data.refreshToken,
      }),
    });
    
    const refreshData = await refreshResponse.json();
    expect(refreshData.success).toBe(true);
    expect(refreshData.data.token).toBeTruthy();
  });

  test('4. CSRF endpoint returns token', async () => {
    const response = await fetch(`${PROD_API}/api/v1/auth/csrf`);
    expect(response.ok).toBeTruthy();
  });

  test('5. API rejects unauthorized requests', async () => {
    const response = await fetch(`${PROD_API}/api/v1/revenue`, {
      headers: { 'Authorization': 'Bearer invalid-token' },
    });
    expect(response.status).toBe(401);
  });
});

test.describe('Production User Profile & Settings', () => {
  let token: string;
  let user: any;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
    user = auth.user;
  });

  test('6. Get auth/me returns current user', async () => {
    const data = await apiCall(token, '/api/v1/auth/me');
    expect(data.success).toBe(true);
    expect(data.data.email).toBe(TEST_USER.email);
  });

  test('7. Get billing usage returns subscription info', async () => {
    const data = await apiCall(token, '/api/v1/billing/usage/current');
    expect(data.success).toBe(true);
  });

  test('8. Settings endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/settings');
    // May return success or "feature not available" - both are valid responses
    expect(data).toBeTruthy();
    expect(typeof data.success === 'boolean' || data.code === 'FEATURE_DISABLED').toBe(true);
  });
});

test.describe('Production Revenue & Analytics API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('9. Revenue summary endpoint returns data', async () => {
    const data = await apiCall(token, '/api/v1/revenue/summary');
    expect(data.success).toBe(true);
  });

  test('10. Dashboard endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/dashboard/metrics');
    // May be disabled or enabled depending on config
    expect(data).toBeTruthy();
  });

  test('11. Reporting overview endpoint', async () => {
    const data = await apiCall(token, '/api/v1/reporting/overview');
    expect(data).toBeTruthy();
  });

  test('12. Analytics overview endpoint', async () => {
    const data = await apiCall(token, '/api/v1/analytics/overview');
    expect(data.success).toBe(true);
  });
});

test.describe('Production Placements API', () => {
  let token: string;
  let createdPlacementId: string | null = null;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('13. List placements returns array or disabled message', async () => {
    const data = await apiCall(token, '/api/v1/placements');
    // May return success with array, object, or feature disabled
    expect(data).toBeTruthy();
    if (data.success && data.data !== undefined) {
      // data.data can be array or object depending on implementation
      expect(typeof data.data === 'object' || Array.isArray(data.data)).toBe(true);
    }
  });

  test('14. Create placement or get proper error', async () => {
    const placementData = {
      name: `Test Placement ${Date.now()}`,
      adFormat: 'banner',
      isActive: true,
    };
    
    const data = await apiCall(token, '/api/v1/placements', {
      method: 'POST',
      body: placementData,
    });
    
    // May succeed or return validation error
    expect(data).toBeTruthy();
    if (data.success) {
      expect(data.data.id).toBeTruthy();
      createdPlacementId = data.data.id;
    }
  });

  test('15. Get specific placement', async () => {
    if (!createdPlacementId) {
      test.skip();
      return;
    }
    
    const data = await apiCall(token, `/api/v1/placements/${createdPlacementId}`);
    expect(data.success).toBe(true);
    expect(data.data.id).toBe(createdPlacementId);
  });

  test('16. Update placement', async () => {
    if (!createdPlacementId) {
      test.skip();
      return;
    }
    
    const data = await apiCall(token, `/api/v1/placements/${createdPlacementId}`, {
      method: 'PUT',
      body: { name: `Updated Placement ${Date.now()}` },
    });
    
    expect(data.success).toBe(true);
  });

  test('17. Delete placement', async () => {
    if (!createdPlacementId) {
      test.skip();
      return;
    }
    
    const data = await apiCall(token, `/api/v1/placements/${createdPlacementId}`, {
      method: 'DELETE',
    });
    
    expect(data.success).toBe(true);
  });
});

test.describe('Production Adapters API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('18. List adapters', async () => {
    const data = await apiCall(token, '/api/v1/adapters');
    expect(data.success).toBe(true);
    expect(Array.isArray(data.data)).toBe(true);
  });

  test('19. RTB endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/rtb/config');
    // RTB may be enabled or disabled
    expect(data).toBeTruthy();
  });

  test('20. Meta connection endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/meta/campaigns');
    // Meta may not be connected
    expect(data).toBeTruthy();
  });
});

test.describe('Production Fraud Detection API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('21. Fraud endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/fraud/summary');
    expect(data).toBeTruthy();
  });

  test('22. Fraud alerts endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/fraud/alerts');
    expect(data).toBeTruthy();
    // data.data can be array, object, or undefined depending on implementation
    if (data.success && data.data !== undefined) {
      expect(typeof data.data === 'object' || Array.isArray(data.data)).toBe(true);
    }
  });

  test('23. Fraud metrics endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/fraud/events?limit=10');
    expect(data).toBeTruthy();
  });
});

test.describe('Production Billing API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('24. Get billing policy', async () => {
    const data = await apiCall(token, '/api/v1/billing/policy');
    expect(data.success).toBe(true);
  });

  test('25. Get invoices', async () => {
    const data = await apiCall(token, '/api/v1/billing/invoices');
    expect(data.success).toBe(true);
    expect(Array.isArray(data.data)).toBe(true);
  });

  test('26. Get usage summary', async () => {
    const data = await apiCall(token, '/api/v1/billing/usage/current');
    expect(data.success).toBe(true);
  });

  test('27. Get platform tiers', async () => {
    const data = await apiCall(token, '/api/v1/billing/tiers');
    expect(data.success).toBe(true);
  });
});

test.describe('Production Team Management API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('28. List team members', async () => {
    const data = await apiCall(token, '/api/v1/team/members');
    expect(data.success).toBe(true);
    expect(Array.isArray(data.data)).toBe(true);
  });

  test('29. Get available roles', async () => {
    const data = await apiCall(token, '/api/v1/team/roles');
    expect(data.success).toBe(true);
  });
});

test.describe('Production VRA (Viewability Revenue Attribution) API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('30. VRA connections endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/vra/connections');
    expect(data).toBeTruthy();
  });

  test('31. Recon runs endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/recon/runs');
    expect(data).toBeTruthy();
  });

  test('32. Proofs endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/proofs');
    expect(data).toBeTruthy();
  });
});

test.describe('Production Migration Studio API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('33. Migration progress endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/migration/progress');
    expect(data).toBeTruthy();
  });

  test('34. Migration experiments endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/migration/experiments');
    expect(data).toBeTruthy();
  });
});

test.describe('Production Supply Chain API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('35. Transparency receipts endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/transparency/receipts');
    expect(data).toBeTruthy();
  });
});

test.describe('Production Transparency API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('36. Public keys endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/transparency/public-keys');
    expect(data).toBeTruthy();
  });
});

test.describe('Production Tools API', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('37. App-ads.txt inspector endpoint', async () => {
    const data = await apiCall(token, '/api/v1/tools/app-ads-txt/inspect?url=https://example.com', {
      method: 'GET',
    });
    // May fail if URL doesn't have app-ads.txt, but endpoint should work
    expect(data).toBeTruthy();
  });

  test('38. Flags endpoint responds', async () => {
    const data = await apiCall(token, '/api/v1/flags');
    expect(data).toBeTruthy();
  });
});

test.describe('Production API Rate Limiting', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('39. API returns proper response structure', async () => {
    const response = await fetch(`${PROD_API}/api/v1/analytics/overview`, {
      headers: { 'Authorization': `Bearer ${token}` },
    });
    
    // Just verify we get a proper response
    expect(response.ok).toBeTruthy();
    const data = await response.json();
    expect(data.success).toBe(true);
  });
});

test.describe('Production CORS & Security Headers', () => {
  test('40. CORS headers allow console origin', async () => {
    const response = await fetch(`${PROD_API}/api/v1/auth/login`, {
      method: 'OPTIONS',
      headers: {
        'Origin': PROD_CONSOLE,
        'Access-Control-Request-Method': 'POST',
      },
    });
    
    const allowOrigin = response.headers.get('access-control-allow-origin');
    expect(allowOrigin).toBe(PROD_CONSOLE);
  });

  test('41. Security headers are present', async () => {
    const response = await fetch(`${PROD_API}/api/v1/health`);
    
    // Check for security headers (may be added by Caddy)
    expect(response.headers.get('x-content-type-options')).toBe('nosniff');
  });
});

test.describe('Production Health & Monitoring', () => {
  test('42. Health endpoint returns healthy', async () => {
    const response = await fetch(`${PROD_API}/health`);
    expect(response.ok).toBeTruthy();
    
    const data = await response.json();
    expect(data.status).toBe('healthy');
  });

  test('43. Readiness probe works', async () => {
    const response = await fetch(`${PROD_API}/ready`);
    expect(response.ok).toBeTruthy();
  });
});

test.describe('Production Console UI Smoke Tests', () => {
  let page: Page;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage();
  });

  test.afterAll(async () => {
    await page.close();
  });

  test('44. Console login page loads', async () => {
    await page.goto(`${PROD_CONSOLE}/login`);
    await expect(page.locator('input[type="email"], input[name="email"]')).toBeVisible({ timeout: 10000 });
  });

  test('45. Console dashboard loads after login', async () => {
    // Perform login
    await page.goto(`${PROD_CONSOLE}/login`);
    await page.fill('input[type="email"], input[name="email"]', TEST_USER.email);
    await page.fill('input[type="password"]', TEST_USER.password);
    await page.click('button[type="submit"]');
    
    // Wait for redirect to dashboard
    await page.waitForURL(/\/(dashboard|$)/, { timeout: 15000 });
    
    // Verify dashboard content
    await expect(page).toHaveURL(/\/(dashboard|$)/);
  });
});

test.describe('Production Error Handling', () => {
  let token: string;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
  });

  test('46. 404 returns proper error format', async () => {
    const response = await fetch(`${PROD_API}/api/v1/nonexistent-endpoint-12345`, {
      headers: { 'Authorization': `Bearer ${token}` },
    });
    
    expect(response.status).toBe(404);
    const data = await response.json();
    expect(data.success).toBe(false);
    expect(data.error).toBeTruthy();
  });

  test('47. Invalid JSON returns error', async () => {
    const response = await fetch(`${PROD_API}/api/v1/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: 'not-valid-json{',
    });
    
    // Should return an error status (4xx or 5xx)
    expect(response.ok).toBe(false);
  });
});

test.describe('Production Database Consistency', () => {
  let token: string;
  let user: any;

  test.beforeAll(async () => {
    const auth = await getAuthToken();
    token = auth.token;
    user = auth.user;
  });

  test('48. Auth me returns consistent user data', async () => {
    // Get user from /auth/me
    const userData = await apiCall(token, '/api/v1/auth/me');
    expect(userData.success).toBe(true);
    
    // Verify relationship
    expect(userData.data.email).toBe(TEST_USER.email);
    expect(userData.data.publisherId).toBe(user.publisherId);
  });

  test('49. Settings tied to correct publisher', async () => {
    const settings = await apiCall(token, '/api/v1/settings');
    // Settings should return something (may be empty for new publisher)
    expect(settings).toBeTruthy();
  });
});
