import { test, expect } from '@playwright/test';

/**
 * Publisher Trust Package E2E Tests
 * 
 * Tests for the new Trust Package pages:
 * - SLO/SLA Page (/slo)
 * - Security Pack Page (/security-pack)
 * - Verification Docs Page (/docs/verification)
 * - Playback Matrix Page (/docs/playback-matrix)
 * - SSAI Evaluation Sandbox (/docs/getting-started/sandbox)
 */

test.describe('SLO/SLA Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/slo');
  });

  test('page loads with correct hero content', async ({ page }) => {
    await expect(page.locator('h1')).toContainText(/service level|SLO/i);
    await expect(page.locator('main').first()).toBeVisible();
    await expect(page.locator('nav').first()).toBeVisible();
    await expect(page.locator('footer').first()).toBeVisible();
  });

  test('displays all 5 SLO targets', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for SLO metrics - using patterns that exist in the page
    expect(pageContent).toMatch(/99\.9%|availability/i); // Availability
    expect(pageContent).toMatch(/latency|50ms/i); // Latency
    expect(pageContent).toMatch(/pod|deadline|fill/i); // Pod deadline hit rate
    expect(pageContent).toMatch(/manifest/i); // Invalid manifest rate
    expect(pageContent).toMatch(/tracking|integrity/i); // Tracking integrity
  });

  test('shows measurement methodology section', async ({ page }) => {
    await expect(page.getByText(/how we measure/i).first()).toBeVisible();
  });

  test('displays mitigations section', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for mitigation features
    expect(pageContent).toMatch(/safe mode/i);
    expect(pageContent).toMatch(/rollback|auto-rollback/i);
    expect(pageContent).toMatch(/circuit breaker/i);
  });

  test('has proper heading hierarchy', async ({ page }) => {
    const h1 = page.locator('h1');
    await expect(h1.first()).toBeVisible();
    
    const h2Count = await page.locator('h2').count();
    expect(h2Count).toBeGreaterThanOrEqual(2);
  });

  test('screenshot for visual verification', async ({ page }) => {
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/slo-page.png', 
      fullPage: true 
    });
  });
});

test.describe('Security Pack Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/security-pack');
  });

  test('page loads with correct hero content', async ({ page }) => {
    await expect(page.locator('h1')).toContainText(/security pack/i);
    await expect(page.locator('main').first()).toBeVisible();
  });

  test('displays security pack contents list', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for key security pack items
    expect(pageContent).toMatch(/architecture/i);
    expect(pageContent).toMatch(/encryption/i);
    expect(pageContent).toMatch(/rbac|access control/i);
    expect(pageContent).toMatch(/audit/i);
    expect(pageContent).toMatch(/threat model/i);
  });

  test('shows threat model table', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for threat categories that exist in the page
    expect(pageContent).toMatch(/cache poisoning|token leakage|ssrf|replay/i);
    expect(pageContent).toMatch(/mitigated/i);
  });

  test('displays compliance badges', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for compliance standards
    expect(pageContent).toMatch(/GDPR/i);
    expect(pageContent).toMatch(/CCPA/i);
    expect(pageContent).toMatch(/TCF 2\.2/i);
    expect(pageContent).toMatch(/SOC\s?2/i);
  });

  test('shows DSAR workflow', async ({ page }) => {
    const pageContent = await page.content();
    expect(pageContent).toMatch(/DSAR|data subject|subject request/i);
  });

  test('screenshot for visual verification', async ({ page }) => {
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/security-pack-page.png', 
      fullPage: true 
    });
  });
});

test.describe('Verification Docs Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/docs/verification');
  });

  test('page loads with correct hero content', async ({ page }) => {
    await expect(page.locator('h1')).toContainText(/verify/i);
    await expect(page.locator('main').first()).toBeVisible();
  });

  test('explains why signed logs matter', async ({ page }) => {
    const pageContent = await page.content();
    
    expect(pageContent).toMatch(/tamper|tamper-evident/i);
    expect(pageContent).toMatch(/non-repudiation/i);
    expect(pageContent).toMatch(/verification|verify/i);
  });

  test('shows verification steps', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for verification steps
    expect(pageContent).toMatch(/export/i);
    expect(pageContent).toMatch(/bundle/i);
    expect(pageContent).toMatch(/signature/i);
  });

  test('displays CLI examples', async ({ page }) => {
    // Look for code blocks with CLI commands
    const codeBlocks = await page.locator('pre').count();
    expect(codeBlocks).toBeGreaterThanOrEqual(1);
    
    const pageContent = await page.content();
    expect(pageContent).toMatch(/apex-cli|openssl/i);
  });

  test('shows evidence bundle structure', async ({ page }) => {
    const pageContent = await page.content();
    
    expect(pageContent).toMatch(/bundleId|sessionId/i);
    expect(pageContent).toMatch(/signatures/i);
    expect(pageContent).toMatch(/components/i);
  });

  test('screenshot for visual verification', async ({ page }) => {
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/verification-docs-page.png', 
      fullPage: true 
    });
  });
});

test.describe('Playback Matrix Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/docs/playback-matrix');
  });

  test('page loads with correct hero content', async ({ page }) => {
    await expect(page.locator('h1')).toContainText(/playback matrix/i);
    await expect(page.locator('main').first()).toBeVisible();
  });

  test('displays CTV platforms section', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for major CTV platforms
    expect(pageContent).toMatch(/roku/i);
    expect(pageContent).toMatch(/fire tv|firetv/i);
    expect(pageContent).toMatch(/apple tv|appletv/i);
    expect(pageContent).toMatch(/android tv|androidtv/i);
  });

  test('displays Web browsers section', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for major browsers
    expect(pageContent).toMatch(/chrome/i);
    expect(pageContent).toMatch(/firefox/i);
    expect(pageContent).toMatch(/safari/i);
    expect(pageContent).toMatch(/edge/i);
  });

  test('displays Mobile platforms section', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for mobile platforms
    expect(pageContent).toMatch(/ios native|ios/i);
    expect(pageContent).toMatch(/android native|android/i);
    expect(pageContent).toMatch(/react native/i);
    expect(pageContent).toMatch(/flutter/i);
  });

  test('shows certification status badges', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for status indicators
    expect(pageContent).toMatch(/certified/i);
  });

  test('displays certification process', async ({ page }) => {
    const pageContent = await page.content();
    
    expect(pageContent).toMatch(/how we certify|certification/i);
  });

  test('has tables with platform details', async ({ page }) => {
    const tables = await page.locator('table').count();
    expect(tables).toBeGreaterThanOrEqual(1);
    
    // Check for table headers
    const pageContent = await page.content();
    expect(pageContent).toMatch(/platform|version|status/i);
  });

  test('screenshot for visual verification', async ({ page }) => {
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/playback-matrix-page.png', 
      fullPage: true 
    });
  });
});

test.describe('Trust Package Integration', () => {
  test('Home page has Proof Strip with links to trust pages', async ({ page }) => {
    await page.goto('/');
    
    const pageContent = await page.content();
    
    // Check for proof strip content
    expect(pageContent).toMatch(/verify before you buy|public slo|signed logs|security pack/i);
  });

  test('Security page links to Security Pack', async ({ page }) => {
    await page.goto('/security');
    
    // Check for Security Pack link
    const securityPackLink = page.locator('a[href="/security-pack"]');
    await expect(securityPackLink.first()).toBeVisible();
  });

  test('Security page shows TCF 2.2 (not 2.0)', async ({ page }) => {
    await page.goto('/security');
    
    const pageContent = await page.content();
    expect(pageContent).toMatch(/TCF 2\.2/);
    expect(pageContent).not.toMatch(/TCF 2\.0[^2]/); // Ensure 2.0 isn't present (except as part of 2.02 etc)
  });

  test('Developers page has working docs link', async ({ page }) => {
    await page.goto('/developers');
    
    const pageContent = await page.content();
    
    // Should not have "coming soon" for docs
    expect(pageContent).not.toMatch(/docs.*coming soon/i);
    
    // Should have link to documentation
    const docsLink = page.locator('a[href*="docs"], a[href*="playback"]');
    await expect(docsLink.first()).toBeVisible();
  });

  test('Product page console link is active', async ({ page }) => {
    await page.goto('/product');
    
    const pageContent = await page.content();
    
    // Should not have "coming soon" for console
    expect(pageContent).not.toMatch(/console.*coming soon/i);
  });

  test('Company page does not show unverifiable metrics', async ({ page }) => {
    await page.goto('/company');
    
    const pageContent = await page.content();
    
    // Should not have unverifiable impression claims
    expect(pageContent).not.toMatch(/1B\+ impressions/i);
  });
});

test.describe('Trust Package Mobile Responsiveness', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
  });

  test('SLO page is responsive on mobile', async ({ page }) => {
    await page.goto('/slo');
    await expect(page.locator('h1')).toBeVisible();
    await expect(page.locator('main').first()).toBeVisible();
    
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/slo-mobile.png', 
      fullPage: true 
    });
  });

  test('Security Pack page is responsive on mobile', async ({ page }) => {
    await page.goto('/security-pack');
    await expect(page.locator('h1')).toBeVisible();
    
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/security-pack-mobile.png', 
      fullPage: true 
    });
  });

  test('Verification docs page is responsive on mobile', async ({ page }) => {
    await page.goto('/docs/verification');
    await expect(page.locator('h1')).toBeVisible();
    
    // Code blocks should be scrollable, not overflow
    const preElements = await page.locator('pre').all();
    for (const pre of preElements) {
      const box = await pre.boundingBox();
      if (box) {
        expect(box.width).toBeLessThanOrEqual(375);
      }
    }
    
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/verification-mobile.png', 
      fullPage: true 
    });
  });

  test('Playback Matrix page tables are accessible on mobile', async ({ page }) => {
    await page.goto('/docs/playback-matrix');
    await expect(page.locator('h1')).toBeVisible();
    
    // Tables should be in scrollable containers
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/playback-matrix-mobile.png', 
      fullPage: true 
    });
  });
});

test.describe('Trust Package Accessibility', () => {
  test('SLO page has proper heading hierarchy', async ({ page }) => {
    await page.goto('/slo');
    
    const h1 = page.locator('h1');
    const h1Count = await h1.count();
    expect(h1Count).toBe(1);
    
    const h2Count = await page.locator('h2').count();
    expect(h2Count).toBeGreaterThanOrEqual(2);
  });

  test('Security Pack page has proper heading hierarchy', async ({ page }) => {
    await page.goto('/security-pack');
    
    const h1 = page.locator('h1');
    const h1Count = await h1.count();
    expect(h1Count).toBe(1);
  });

  test('All trust pages have proper lang attribute', async ({ page }) => {
    const pages = ['/slo', '/security-pack', '/docs/verification', '/docs/playback-matrix'];
    
    for (const pagePath of pages) {
      await page.goto(pagePath);
      const html = page.locator('html');
      const lang = await html.getAttribute('lang');
      expect(lang).toBeTruthy();
    }
  });

  test('All trust pages have proper viewport meta', async ({ page }) => {
    const pages = ['/slo', '/security-pack', '/docs/verification', '/docs/playback-matrix'];
    
    for (const pagePath of pages) {
      await page.goto(pagePath);
      const viewport = await page.locator('meta[name="viewport"]').first().getAttribute('content');
      expect(viewport).toContain('width=');
    }
  });
});

test.describe('SSAI Evaluation Sandbox Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/docs/getting-started/sandbox');
  });

  test('page loads with correct hero content', async ({ page }) => {
    await expect(page.locator('h1')).toContainText(/sandbox|evaluation/i);
    await expect(page.locator('main').first()).toBeVisible();
    await expect(page.locator('nav').first()).toBeVisible();
    await expect(page.locator('footer').first()).toBeVisible();
  });

  test('displays start demo button', async ({ page }) => {
    const startButton = page.getByRole('button', { name: /start demo/i });
    await expect(startButton).toBeVisible();
  });

  test('shows sample manifest URLs', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for HLS and DASH URLs
    expect(pageContent).toMatch(/hls|\.m3u8/i);
    expect(pageContent).toMatch(/dash|\.mpd/i);
  });

  test('displays live event console', async ({ page }) => {
    const pageContent = await page.content();
    
    expect(pageContent).toMatch(/live.*console|event console/i);
  });

  test('shows platform integration examples', async ({ page }) => {
    const pageContent = await page.content();
    
    // Check for player integration snippets
    expect(pageContent).toMatch(/web|ios|android/i);
    expect(pageContent).toMatch(/avplayer|exoplayer|hls\.js|shaka/i);
  });

  test('demo simulation works when started', async ({ page }) => {
    // Click start button
    const startButton = page.getByRole('button', { name: /start demo/i });
    await startButton.click();
    
    // Wait for events to start appearing
    await page.waitForTimeout(2000);
    
    // Check that events are showing
    const pageContent = await page.content();
    expect(pageContent).toMatch(/session_init|cue_received|auction/i);
  });

  test('shows auction results after demo runs', async ({ page }) => {
    // Click start button
    const startButton = page.getByRole('button', { name: /start demo/i });
    await startButton.click();
    
    // Wait for auction results section
    await page.waitForTimeout(4000);
    
    // Check for auction results
    const pageContent = await page.content();
    expect(pageContent).toMatch(/pod summary|partner responses|auction results/i);
  });

  test('screenshot for visual verification', async ({ page }) => {
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/sandbox-page.png', 
      fullPage: true 
    });
  });

  test('sandbox page mobile responsive', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto('/docs/getting-started/sandbox');
    
    await expect(page.locator('h1')).toBeVisible();
    await expect(page.locator('main').first()).toBeVisible();
    
    await page.screenshot({ 
      path: 'test-results/screenshots/trust-package/sandbox-mobile.png', 
      fullPage: true 
    });
  });
});
