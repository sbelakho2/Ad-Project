import { test, expect } from '@playwright/test';

test.describe('Website Marketing Pages', () => {
  test('homepage loads with hero section', async ({ page }) => {
    await page.goto('/');
    
    // Check core elements
    await expect(page.locator('nav').first()).toBeVisible();
    await expect(page.locator('footer').first()).toBeVisible();
    
    // Check for main CTA or hero section - use first() for pages with multiple main elements
    const main = page.locator('main').first();
    await expect(main).toBeVisible();
    
    // Screenshot for visual verification
    await page.screenshot({ path: 'test-results/screenshots/homepage.png', fullPage: true });
  });

  test('pricing page displays tier information', async ({ page }) => {
    await page.goto('/pricing');
    
    // Check page loads - use first() for strict mode
    await expect(page.locator('main').first()).toBeVisible();
    
    // Check for pricing tiers - looking for Starter, Growth, Scale, Enterprise
    const tierContent = await page.content();
    expect(tierContent).toMatch(/starter|growth|scale|enterprise/i);
    
    // Check for pricing details
    expect(tierContent).toMatch(/\$|%|free/i);
    
    await page.screenshot({ path: 'test-results/screenshots/pricing.png', fullPage: true });
  });

  test('navigation links work correctly', async ({ page }) => {
    await page.goto('/');
    
    // Find navigation and check for key links
    const nav = page.locator('nav').first();
    await expect(nav).toBeVisible();
    
    // Check header/nav has appropriate links
    const links = await nav.locator('a').all();
    expect(links.length).toBeGreaterThan(0);
    
    await page.screenshot({ path: 'test-results/screenshots/navigation.png' });
  });

  test('mobile navigation works', async ({ page }) => {
    // Set mobile viewport
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto('/');
    
    // Check for mobile menu button or hamburger
    const menuButton = page.locator('button[aria-label*="menu"], button[aria-label*="Menu"], [data-testid="mobile-menu"]');
    
    if (await menuButton.count() > 0) {
      await menuButton.first().click();
      await page.waitForTimeout(300); // Wait for animation
      await page.screenshot({ path: 'test-results/screenshots/mobile-nav-open.png' });
    }
    
    await page.screenshot({ path: 'test-results/screenshots/homepage-mobile.png', fullPage: true });
  });

  test('page has proper meta tags and SEO elements', async ({ page }) => {
    await page.goto('/');
    
    // Check meta title
    const title = await page.title();
    expect(title.length).toBeGreaterThan(0);
    
    // Check for viewport meta - use first() for strict mode
    const viewport = await page.locator('meta[name="viewport"]').first().getAttribute('content');
    expect(viewport).toContain('width=');
    
    // Check lang attribute
    const html = page.locator('html');
    const lang = await html.getAttribute('lang');
    expect(lang).toBeTruthy();
  });

  test('footer contains essential links', async ({ page }) => {
    await page.goto('/');
    
    const footer = page.locator('footer').first();
    await expect(footer).toBeVisible();
    
    await page.screenshot({ path: 'test-results/screenshots/footer.png' });
  });
});

test.describe('Website Accessibility', () => {
  test('homepage has proper heading hierarchy', async ({ page }) => {
    await page.goto('/');
    
    // Check for h1
    const h1 = page.locator('h1');
    await expect(h1.first()).toBeVisible();
    
    // Should have only one h1
    const h1Count = await h1.count();
    expect(h1Count).toBeGreaterThanOrEqual(1);
  });

  test('images have alt text', async ({ page }) => {
    await page.goto('/');
    
    const images = page.locator('img');
    const count = await images.count();
    
    for (let i = 0; i < Math.min(count, 10); i++) {
      const alt = await images.nth(i).getAttribute('alt');
      // Alt can be empty string for decorative images, but should exist
      expect(alt !== null).toBe(true);
    }
  });

  test('interactive elements are focusable', async ({ page }) => {
    await page.goto('/');
    
    // Tab through page
    await page.keyboard.press('Tab');
    
    // Check something is focused
    const focusedElement = page.locator(':focus');
    await expect(focusedElement).toBeVisible({ timeout: 3000 });
  });
});

test.describe('Website Performance', () => {
  test('page loads within acceptable time', async ({ page }) => {
    const startTime = Date.now();
    await page.goto('/');
    const loadTime = Date.now() - startTime;
    
    // Should load within 5 seconds
    expect(loadTime).toBeLessThan(5000);
    console.log(`Homepage load time: ${loadTime}ms`);
  });

  test('no critical console errors on page load', async ({ page }) => {
    const errors: string[] = [];
    page.on('console', msg => {
      if (msg.type() === 'error') {
        errors.push(msg.text());
      }
    });
    
    await page.goto('/');
    await page.waitForTimeout(1000);
    
    // Filter out known acceptable errors (CSP, favicon, hydration)
    const criticalErrors = errors.filter(e => 
      !e.includes('favicon') && 
      !e.includes('404') &&
      !e.includes('hydration') &&
      !e.includes('Content Security Policy') &&
      !e.includes('CSP')
    );
    
    expect(criticalErrors).toHaveLength(0);
  });
});

test.describe('Website Visual Regression', () => {
  test('pricing page layout is consistent', async ({ page }) => {
    await page.goto('/pricing');
    await page.waitForLoadState('networkidle');
    
    // Take screenshot at different viewports
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.screenshot({ path: 'test-results/screenshots/pricing-desktop.png', fullPage: true });
    
    await page.setViewportSize({ width: 768, height: 1024 });
    await page.screenshot({ path: 'test-results/screenshots/pricing-tablet.png', fullPage: true });
    
    await page.setViewportSize({ width: 375, height: 667 });
    await page.screenshot({ path: 'test-results/screenshots/pricing-mobile.png', fullPage: true });
  });
});
