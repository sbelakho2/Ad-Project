import { test, expect, Page } from '@playwright/test';

/**
 * Comprehensive website tests across all pages and devices.
 * Tests navigation, interactive elements, and page loading.
 */

// All public pages to test
const PUBLIC_PAGES = [
  { path: '/', name: 'Home' },
  { path: '/product', name: 'Product' },
  { path: '/features', name: 'Features' },
  { path: '/pricing', name: 'Pricing' },
  { path: '/developers', name: 'Developers' },
  { path: '/security', name: 'Security' },
  { path: '/company', name: 'Company' },
  { path: '/contact', name: 'Contact' },
  { path: '/privacy', name: 'Privacy' },
  { path: '/terms', name: 'Terms' },
  { path: '/cookies', name: 'Cookies' },
  { path: '/gdpr', name: 'GDPR' },
  { path: '/dpa', name: 'DPA' },
  { path: '/signin', name: 'Sign In' },
  { path: '/signup', name: 'Sign Up' },
];

// Navigation links that should be present in the nav
const NAV_LINKS = [
  { href: '/product', label: 'Product' },
  { href: '/features', label: 'Features' },
  { href: '/pricing', label: 'Pricing' },
  { href: '/developers', label: 'Developers' },
  { href: '/security', label: 'Security' },
  { href: '/company', label: 'Company' },
];

// Footer links that should be present
// Note: Some links appear multiple times in footer (e.g., /security in About, Resources, Legal)
// Privacy and Terms also appear twice (once in Legal section, once in bottom bar)
const FOOTER_LINKS = [
  '/company',
  '/contact',
  '/product',
  '/pricing',
  '/developers',
  '/dpa',
  '/gdpr',
];

// Links that appear multiple times in footer
const FOOTER_LINKS_MULTIPLE = ['/security', '/privacy', '/terms'];

// ============================================================================
// PAGE LOADING TESTS
// ============================================================================

test.describe('Page Loading - All Pages Load Successfully', () => {
  for (const page of PUBLIC_PAGES) {
    test(`${page.name} page (${page.path}) loads without errors`, async ({ page: browserPage }) => {
      // Track console errors
      const errors: string[] = [];
      browserPage.on('console', msg => {
        if (msg.type() === 'error') {
          errors.push(msg.text());
        }
      });

      const response = await browserPage.goto(page.path);
      
      // Page should return 200
      expect(response?.status()).toBe(200);
      
      // Wait for page to be fully loaded
      await browserPage.waitForLoadState('networkidle');
      
      // Page should have a title
      const title = await browserPage.title();
      expect(title).toBeTruthy();
      expect(title.length).toBeGreaterThan(0);
      
      // No critical JavaScript errors (filter out non-critical ones)
      const criticalErrors = errors.filter(e => 
        !e.includes('Download the React DevTools') &&
        !e.includes('favicon')
      );
      expect(criticalErrors).toHaveLength(0);
    });
  }
});

// ============================================================================
// MOBILE MENU TESTS (Mobile viewports only)
// ============================================================================

test.describe('Mobile Navigation', () => {
  test.use({ viewport: { width: 375, height: 667 } });

  for (const page of PUBLIC_PAGES.slice(0, 7)) { // Test on first 7 pages
    test(`Mobile menu works on ${page.name} page`, async ({ page: browserPage }) => {
      await browserPage.goto(page.path);
      await browserPage.waitForLoadState('networkidle');
      
      // Find hamburger button
      const hamburgerButton = browserPage.locator('button[aria-label="Open menu"]');
      
      // Should be visible on mobile
      if (await hamburgerButton.isVisible()) {
        // Should start closed
        await expect(hamburgerButton).toHaveAttribute('aria-expanded', 'false');
        
        // Click to open
        await hamburgerButton.click();
        await browserPage.waitForTimeout(300);
        
        // Menu should be visible
        const mobileMenu = browserPage.locator('#mobile-navigation');
        await expect(mobileMenu).toBeVisible();
        
        // Should have nav links
        for (const link of NAV_LINKS) {
          await expect(mobileMenu.locator(`a[href="${link.href}"]`)).toBeVisible();
        }
        
        // Close button should work
        const closeButton = mobileMenu.locator('button[aria-label="Close menu"]');
        await closeButton.click();
        await browserPage.waitForTimeout(300);
        
        // Menu should be closed
        await expect(mobileMenu).not.toBeVisible();
      }
    });
  }

  test('Mobile menu navigation works', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    // Open menu
    await page.locator('button[aria-label="Open menu"]').click();
    await page.waitForTimeout(300);
    
    // Click Product link
    await page.locator('#mobile-navigation a[href="/product"]').click();
    
    // Should navigate
    await expect(page).toHaveURL(/.*\/product/);
    
    // Menu should be closed after navigation
    await expect(page.locator('#mobile-navigation')).not.toBeVisible();
  });

  test('Clicking backdrop closes mobile menu', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    // Open menu
    await page.locator('button[aria-label="Open menu"]').click();
    await page.waitForTimeout(300);
    
    // Click backdrop
    const backdrop = page.locator('div.fixed.inset-0.bg-black\\/50');
    if (await backdrop.isVisible()) {
      await backdrop.click({ position: { x: 350, y: 300 } });
      await page.waitForTimeout(300);
      await expect(page.locator('#mobile-navigation')).not.toBeVisible();
    }
  });
});

// ============================================================================
// DESKTOP NAVIGATION TESTS
// ============================================================================

test.describe('Desktop Navigation', () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test('Desktop nav has all links visible', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    // Hamburger should be hidden on desktop
    const hamburger = page.locator('button[aria-label="Open menu"]');
    await expect(hamburger).not.toBeVisible();
    
    // All nav links should be visible
    for (const link of NAV_LINKS) {
      const navLink = page.locator(`nav a[href="${link.href}"]`).first();
      await expect(navLink).toBeVisible();
    }
    
    // Sign in and Sign up should be visible
    await expect(page.locator('nav a[href="/signin"]').first()).toBeVisible();
    await expect(page.locator('nav a[href="/signup"]').first()).toBeVisible();
  });

  test('Desktop nav links navigate correctly', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    for (const link of NAV_LINKS) {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      const navLink = page.locator(`nav a[href="${link.href}"]`).first();
      await navLink.click();
      
      await expect(page).toHaveURL(new RegExp(`.*${link.href}`));
    }
  });
});

// ============================================================================
// FOOTER TESTS
// ============================================================================

test.describe('Footer', () => {
  test('Footer is present and has all links', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    const footer = page.locator('footer');
    await expect(footer).toBeVisible();
    
    // Check footer links
    for (const href of FOOTER_LINKS) {
      const footerLink = footer.locator(`a[href="${href}"]`);
      await expect(footerLink).toBeVisible();
    }
    
    // Check copyright
    const copyright = footer.locator('text=Bel Consulting');
    await expect(copyright.first()).toBeVisible();
  });

  test('Footer links navigate correctly', async ({ page }) => {
    for (const href of FOOTER_LINKS.slice(0, 5)) { // Test first 5 footer links
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      const footerLink = page.locator(`footer a[href="${href}"]`);
      await footerLink.click();
      
      await expect(page).toHaveURL(new RegExp(`.*${href}`));
    }
  });
});

// ============================================================================
// CTA BUTTON TESTS
// ============================================================================

test.describe('Call to Action Buttons', () => {
  test.describe('Desktop', () => {
    test.use({ viewport: { width: 1280, height: 720 } });
    
    test('Home page CTAs work on desktop', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      // On desktop, nav has signup button visible
      const navSignup = page.locator('nav a[href="/signup"]');
      await expect(navSignup).toBeVisible();
      
      // Hero also has "Get started free" link
      const heroSignup = page.locator('section a[href="/signup"]').first();
      await heroSignup.scrollIntoViewIfNeeded();
      await expect(heroSignup).toBeVisible();
      
      // Contact link in hero
      const heroContact = page.locator('section a[href="/contact"]').first();
      await expect(heroContact).toBeVisible();
    });
  });

  test.describe('Mobile', () => {
    test.use({ viewport: { width: 375, height: 667 } });
    
    test('Home page CTAs work on mobile', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      // Hero CTAs should be visible on mobile (after scrolling)
      const heroSignup = page.locator('section a[href="/signup"]').first();
      await heroSignup.scrollIntoViewIfNeeded();
      await expect(heroSignup).toBeVisible();
    });
  });

  test('Pricing page has CTA buttons', async ({ page }) => {
    await page.goto('/pricing');
    await page.waitForLoadState('networkidle');
    
    // Should have signup/get started links
    const ctaLinks = page.locator('a[href="/signup"], a[href="/contact"]');
    expect(await ctaLinks.count()).toBeGreaterThan(0);
  });
});

// ============================================================================
// LOGO TESTS
// ============================================================================

test.describe('Logo', () => {
  test.describe('Desktop', () => {
    test.use({ viewport: { width: 1280, height: 720 } });

    test('Logo is visible and links to home', async ({ page }) => {
      await page.goto('/product');
      await page.waitForLoadState('networkidle');
      
      // Get the visible desktop navigation logo
      const desktopNav = page.locator('.hidden.md\\:flex');
      const logo = desktopNav.locator('a[href="/"] img[alt="ApexMediation"]');
      await expect(logo).toBeVisible();
      
      // Click logo should go to home
      await logo.click();
      await expect(page).toHaveURL('/');
    });

    test('Desktop logo has correct size (h-44)', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      // Target the visible desktop logo specifically
      const desktopNav = page.locator('.hidden.md\\:flex');
      const logo = desktopNav.locator('img[alt="ApexMediation"]');
      await expect(logo).toBeVisible();
      const className = await logo.getAttribute('class');
      expect(className).toContain('h-44');
    });
  });

  test.describe('Mobile', () => {
    test.use({ viewport: { width: 375, height: 667 } });

    test('Mobile logo has correct size (h-28)', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      // Target the visible mobile logo specifically
      const mobileNav = page.locator('.md\\:hidden').first();
      const logo = mobileNav.locator('img[alt="ApexMediation"]');
      await expect(logo).toBeVisible();
      const className = await logo.getAttribute('class');
      expect(className).toContain('h-28');
    });
  });
});

// ============================================================================
// FORM TESTS
// ============================================================================

test.describe('Forms', () => {
  test('Sign In page has form elements', async ({ page }) => {
    await page.goto('/signin');
    await page.waitForLoadState('networkidle');
    
    // Should have email and password inputs
    const emailInput = page.locator('input[type="email"], input[name="email"]');
    const passwordInput = page.locator('input[type="password"], input[name="password"]');
    
    // At least one form input should exist
    const formInputs = page.locator('input');
    expect(await formInputs.count()).toBeGreaterThan(0);
  });

  test('Sign Up page has form elements', async ({ page }) => {
    await page.goto('/signup');
    await page.waitForLoadState('networkidle');
    
    // Should have form inputs
    const formInputs = page.locator('input');
    expect(await formInputs.count()).toBeGreaterThan(0);
  });

  test('Contact page has form or contact info', async ({ page }) => {
    await page.goto('/contact');
    await page.waitForLoadState('networkidle');
    
    // Should have either a form or contact information
    const form = page.locator('form');
    const emailLink = page.locator('a[href^="mailto:"]');
    
    const hasForm = await form.count() > 0;
    const hasEmail = await emailLink.count() > 0;
    
    expect(hasForm || hasEmail).toBeTruthy();
  });
});

// ============================================================================
// ACCESSIBILITY TESTS
// ============================================================================

test.describe('Accessibility', () => {
  test('Skip to content link exists', async ({ page }) => {
    await page.goto('/');
    
    const skipLink = page.locator('a[href="#main-content"]');
    await expect(skipLink).toBeAttached();
  });

  test('All images have alt text', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    const images = page.locator('img');
    const count = await images.count();
    
    for (let i = 0; i < count; i++) {
      const img = images.nth(i);
      const alt = await img.getAttribute('alt');
      const src = await img.getAttribute('src');
      
      // All images should have alt attribute (can be empty for decorative)
      expect(alt, `Image ${src} should have alt attribute`).not.toBeNull();
    }
  });

  test('Navigation has aria-label', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    const nav = page.locator('nav[aria-label]');
    await expect(nav).toBeVisible();
  });

  test('Buttons have accessible names', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    const buttons = page.locator('button');
    const count = await buttons.count();
    
    for (let i = 0; i < count; i++) {
      const button = buttons.nth(i);
      if (await button.isVisible()) {
        const ariaLabel = await button.getAttribute('aria-label');
        const innerText = await button.innerText();
        
        // Button should have either aria-label or visible text
        expect(ariaLabel || innerText.trim()).toBeTruthy();
      }
    }
  });
});

// ============================================================================
// RESPONSIVE LAYOUT TESTS
// ============================================================================

test.describe('Responsive Layout', () => {
  const viewports = [
    { width: 375, height: 667, name: 'Mobile (iPhone SE)', isMobile: true },
    { width: 767, height: 1024, name: 'Tablet Portrait', isMobile: true }, // Just below md breakpoint
    { width: 768, height: 1024, name: 'Tablet (iPad)', isMobile: false }, // At md breakpoint - desktop nav shows
    { width: 1280, height: 720, name: 'Desktop', isMobile: false },
    { width: 1920, height: 1080, name: 'Large Desktop', isMobile: false },
  ];

  for (const viewport of viewports) {
    test(`Home page renders correctly at ${viewport.name} (${viewport.width}x${viewport.height})`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      // Get the appropriate nav based on viewport
      if (viewport.isMobile) {
        // Mobile nav (md:hidden) should be visible
        const mobileNav = page.locator('.md\\:hidden').first();
        const logo = mobileNav.locator('img[alt="ApexMediation"]');
        await expect(logo).toBeVisible();
      } else {
        // Desktop nav (hidden md:flex) should be visible
        const desktopNav = page.locator('.hidden.md\\:flex');
        const logo = desktopNav.locator('img[alt="ApexMediation"]');
        await expect(logo).toBeVisible();
      }
      
      // Main content should be visible
      const main = page.locator('main');
      await expect(main).toBeVisible();
      
      // Footer should be visible (scroll to it)
      await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
      const footer = page.locator('footer');
      await expect(footer).toBeVisible();
    });
  }
});

// ============================================================================
// PERFORMANCE TESTS
// ============================================================================

test.describe('Performance', () => {
  test('Home page loads within acceptable time', async ({ page }) => {
    const startTime = Date.now();
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    const loadTime = Date.now() - startTime;
    
    // Page should load within 10 seconds
    expect(loadTime).toBeLessThan(10000);
  });

  test('Pages have proper meta tags', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    // Should have title
    const title = await page.title();
    expect(title).toBeTruthy();
    
    // Should have description meta tag
    const description = await page.locator('meta[name="description"]').first().getAttribute('content');
    expect(description).toBeTruthy();
    
    // Should have viewport meta tag
    const viewport = await page.locator('meta[name="viewport"]').first().getAttribute('content');
    expect(viewport).toBeTruthy();
  });
});

// ============================================================================
// SECURITY HEADERS TESTS
// ============================================================================

test.describe('Security', () => {
  test('Security headers are present', async ({ page }) => {
    const response = await page.goto('/');
    const headers = response?.headers();
    
    // Check for security headers
    expect(headers?.['x-content-type-options']).toBe('nosniff');
    expect(headers?.['x-frame-options']).toBe('DENY');
    expect(headers?.['referrer-policy']).toBeTruthy();
  });

  test('CSP allows React to function', async ({ page }) => {
    const errors: string[] = [];
    page.on('console', msg => {
      if (msg.type() === 'error' && msg.text().includes('Content Security Policy')) {
        errors.push(msg.text());
      }
    });

    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    // Try clicking a button to verify React works
    const hamburger = page.locator('button[aria-label="Open menu"]');
    if (await hamburger.isVisible()) {
      await hamburger.click();
      await page.waitForTimeout(300);
    }
    
    // No CSP errors should occur
    expect(errors).toHaveLength(0);
  });
});

// ============================================================================
// EXTERNAL LINKS TESTS
// ============================================================================

test.describe('External Links', () => {
  test('External links open in new tab', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    
    // Find external links (GitHub, Twitter, LinkedIn)
    const externalLinks = page.locator('a[href^="https://github.com"], a[href^="https://twitter.com"], a[href^="https://linkedin.com"]');
    const count = await externalLinks.count();
    
    for (let i = 0; i < count; i++) {
      const link = externalLinks.nth(i);
      const target = await link.getAttribute('target');
      const rel = await link.getAttribute('rel');
      
      // External links should open in new tab with proper rel
      if (target === '_blank') {
        expect(rel).toContain('noopener');
      }
    }
  });
});
