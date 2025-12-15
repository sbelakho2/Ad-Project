import { test, expect } from '@playwright/test';

test.describe('Mobile Menu', () => {
  test.use({ viewport: { width: 375, height: 667 } }); // iPhone SE viewport

  test('hamburger menu opens and closes on click', async ({ page }) => {
    // Capture console logs
    const logs: string[] = [];
    page.on('console', msg => {
      logs.push(`[${msg.type()}] ${msg.text()}`);
    });

    // Go to homepage
    await page.goto('/');

    // Wait for page to be fully hydrated
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(1000); // Extra wait for React hydration

    // Find the hamburger button
    const hamburgerButton = page.locator('button[aria-label="Open menu"]');
    
    // Verify hamburger button is visible
    await expect(hamburgerButton).toBeVisible();
    
    // Verify menu is initially closed (aria-expanded should be false)
    await expect(hamburgerButton).toHaveAttribute('aria-expanded', 'false');

    console.log('Before click - Console logs:', logs);

    // Click the hamburger button
    await hamburgerButton.click();

    // Wait for React state to update
    await page.waitForTimeout(500);

    console.log('After click - Console logs:', logs);

    // The mobile menu panel should now be visible
    const mobileMenu = page.locator('#mobile-navigation');
    await expect(mobileMenu).toBeVisible();

    // Verify the menu contains navigation links
    await expect(mobileMenu.locator('a[href="/product"]')).toBeVisible();
    await expect(mobileMenu.locator('a[href="/features"]')).toBeVisible();
    await expect(mobileMenu.locator('a[href="/pricing"]')).toBeVisible();

    // Find and click the close button
    const closeButton = mobileMenu.locator('button[aria-label="Close menu"]');
    await expect(closeButton).toBeVisible();
    await closeButton.click();

    // Wait for menu to close
    await page.waitForTimeout(300);

    // Menu should no longer be visible
    await expect(mobileMenu).not.toBeVisible();
  });

  test('clicking a menu link navigates and closes menu', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    // Open the menu
    const hamburgerButton = page.locator('button[aria-label="Open menu"]');
    await hamburgerButton.click();
    await page.waitForTimeout(300);

    // Click on Product link
    const productLink = page.locator('#mobile-navigation a[href="/product"]');
    await expect(productLink).toBeVisible();
    await productLink.click();

    // Should navigate to /product
    await expect(page).toHaveURL(/.*\/product/);

    // Menu should be closed after navigation
    const mobileMenu = page.locator('#mobile-navigation');
    await expect(mobileMenu).not.toBeVisible();
  });

  test('clicking backdrop closes menu', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    // Open the menu
    const hamburgerButton = page.locator('button[aria-label="Open menu"]');
    await hamburgerButton.click();
    await page.waitForTimeout(300);

    // Menu should be visible
    const mobileMenu = page.locator('#mobile-navigation');
    await expect(mobileMenu).toBeVisible();

    // Click on the backdrop (the dark overlay area)
    // The backdrop is a div with fixed position covering the screen
    const backdrop = page.locator('div.fixed.inset-0.bg-black\\/50');
    await backdrop.click({ position: { x: 350, y: 300 } }); // Click on the right side where backdrop is visible

    await page.waitForTimeout(300);

    // Menu should be closed
    await expect(mobileMenu).not.toBeVisible();
  });
});

test.describe('Desktop Navigation', () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test('hamburger menu is hidden on desktop', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    // Hamburger button should not be visible on desktop
    const hamburgerButton = page.locator('button[aria-label="Open menu"]');
    await expect(hamburgerButton).not.toBeVisible();

    // Desktop nav links should be visible
    await expect(page.locator('nav a[href="/product"]').first()).toBeVisible();
    await expect(page.locator('nav a[href="/features"]').first()).toBeVisible();
  });
});
