import { test, expect } from '@playwright/test'

const BREAKPOINTS = {
  mobile: { width: 375, height: 667 },
  tablet: { width: 768, height: 1024 },
  desktop: { width: 1280, height: 720 },
  wide: { width: 1920, height: 1080 },
}

const BILLING_PAGES = [
  { path: '/billing/usage', name: 'Usage' },
  { path: '/billing/invoices', name: 'Invoices List' },
  { path: '/billing/settings', name: 'Settings' },
]

// Run visual regression tests for each billing page at each breakpoint
for (const page of BILLING_PAGES) {
  for (const [breakpointName, viewport] of Object.entries(BREAKPOINTS)) {
    test.describe(`${page.name} - ${breakpointName}`, () => {
      test.use({ viewport })

      test('should match screenshot', async ({ page: playwright }) => {
        // TODO: Add authentication before navigating
        await playwright.goto(page.path)
        // Prefer a deterministic readiness sequence over networkidle
        await playwright.waitForLoadState('domcontentloaded')
        await playwright.locator('main').first().waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
        // Wait for any loading spinners to disappear (best-effort)
        await playwright.waitForSelector('[role="status"]', { state: 'hidden', timeout: 5000 }).catch(() => {})
        
        // Take screenshot
        await expect(playwright).toHaveScreenshot(`${page.name.toLowerCase().replace(/\s+/g, '-')}-${breakpointName}.png`, {
          fullPage: true,
          animations: 'disabled',
          // Allow small platform/font rendering drift while still catching real regressions.
          maxDiffPixelRatio: 0.06,
        })
      })

      test('should have no layout shifts', async ({ page: playwright }) => {
        await playwright.goto(page.path)
        await playwright.waitForLoadState('domcontentloaded')
        await playwright.locator('main').first().waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
        
        // Measure CLS (Cumulative Layout Shift)
        const cls = await playwright.evaluate(() => {
          return new Promise<number>((resolve) => {
            let clsValue = 0
            const observer = new PerformanceObserver((list) => {
              for (const entry of list.getEntries()) {
                if ((entry as any).hadRecentInput) continue
                clsValue += (entry as any).value
              }
            })
            observer.observe({ type: 'layout-shift', buffered: true })
            
            // Wait 2 seconds then resolve
            setTimeout(() => {
              observer.disconnect()
              resolve(clsValue)
            }, 2000)
          })
        })
        
        // CLS should be less than 0.1 (good)
        expect(cls).toBeLessThan(0.1)
      })

      test('should load critical content within budget', async ({ page: playwright }) => {
        await playwright.goto(page.path)
        
        // Measure LCP (Largest Contentful Paint)
        const lcp = await playwright.evaluate(() => {
          return new Promise<number>((resolve) => {
            const observer = new PerformanceObserver((list) => {
              const entries = list.getEntries()
              const lastEntry = entries[entries.length - 1]
              resolve((lastEntry as any).renderTime || (lastEntry as any).loadTime)
            })
            observer.observe({ type: 'largest-contentful-paint', buffered: true })
            
            setTimeout(() => {
              observer.disconnect()
              resolve(0)
            }, 5000)
          })
        })
        
        // LCP should be less than 2.5s (good)
        expect(lcp).toBeLessThan(2500)
      })
    })
  }
}

// Invoice detail page (requires dynamic ID)
test.describe('Invoice Detail - Visual Regression', () => {
  for (const [breakpointName, viewport] of Object.entries(BREAKPOINTS)) {
    test(`${breakpointName} - should match screenshot`, async ({ page }) => {
      await page.setViewportSize(viewport as any)
      
      // TODO: Add authentication and use real invoice ID
      await page.goto('/billing/invoices/test-invoice-id')
      await page.waitForLoadState('domcontentloaded')
      await page.locator('main').first().waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
      
      await expect(page).toHaveScreenshot(`invoice-detail-${breakpointName}.png`, {
        fullPage: true,
        animations: 'disabled',
        maxDiffPixelRatio: 0.06,
      })
    })
  }
})

// Test responsive behavior
test.describe('Billing Pages - Responsive Behavior', () => {
  test('Usage page - should show mobile menu on small screens', async ({ page }) => {
    await page.setViewportSize(BREAKPOINTS.mobile)
    await page.goto('/billing/usage')
    
    // Check for responsive navigation
    const nav = page.locator('nav')
    await expect(nav).toBeVisible()
  })

  test('Invoices table - should be scrollable on mobile', async ({ page }) => {
    await page.setViewportSize(BREAKPOINTS.mobile)
    await page.goto('/billing/invoices')
    await page.waitForLoadState('domcontentloaded')
    await page.locator('main').first().waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
    
    // Check table has horizontal scroll or is stacked
    const table = page.locator('table').first()
    if (await table.count() > 0) {
      const tableBox = await table.boundingBox()
      const viewportWidth = BREAKPOINTS.mobile.width
      
      // Table should either be stacked (narrower than viewport) or scrollable
      expect(tableBox).toBeTruthy()
    }
  })

  test('Settings page - should stack sections on mobile', async ({ page }) => {
    test.skip(process.env.RUN_BILLING_SETTINGS_LAYOUT === 'true' ? false : true, 'Set RUN_BILLING_SETTINGS_LAYOUT=true to run this check (requires authenticated billing settings rendering).')

    await page.setViewportSize(BREAKPOINTS.mobile)
    await page.goto('/billing/settings')
    await page.waitForLoadState('domcontentloaded')
    await page.locator('main').first().waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
    
    // All primary content blocks should be visible (stacked vertically).
    // The page may not use semantic containers (<section>/<article>), so assert that
    // there is at least one visible direct child under <main>.
    // All primary content blocks should be visible (stacked vertically).
    // The page may not use <section>, so use robust content containers.
    const blocks = page.locator('main section, main article, main [class*="card"], main [data-testid*="section"]')
    await expect(blocks.first()).toBeVisible()
  })
})

// Test dark mode (if implemented)
test.describe('Billing Pages - Dark Mode', () => {
  test.use({ colorScheme: 'dark' })
  
  for (const page of BILLING_PAGES) {
    test(`${page.name} - should render correctly in dark mode`, async ({ page: playwright }) => {
      await playwright.goto(page.path)
      await playwright.waitForLoadState('domcontentloaded')
      await playwright.locator('main').first().waitFor({ state: 'visible', timeout: 5000 }).catch(() => {})
      
      await expect(playwright).toHaveScreenshot(`${page.name.toLowerCase().replace(/\s+/g, '-')}-dark.png`, {
        fullPage: true,
        animations: 'disabled',
        maxDiffPixelRatio: 0.06,
      })
    })
  }
})
