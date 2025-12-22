import { test, expect } from '@playwright/test'

// Smoke test for Billing Settings save wiring.
// The legacy /settings/payouts route no longer exists; Billing Settings live at:
//   /settings/billing/settings
// This test stubs network calls and validates:
// - Settings load via GET /billing/settings and GET /billing/policy
// - Preference toggles dirty the form
// - Discard Changes does not send PUT
// - Save Changes sends PUT /billing/settings/preferences and shows success toast

// Helpers to recognize endpoints regardless of origin/prefix
function isPath(url: string, path: string) {
  try {
    const u = new URL(url)
    return u.pathname.endsWith(path)
  } catch {
    return url.endsWith(path)
  }
}

function isApiPath(url: string, path: string) {
  try {
    const u = new URL(url)
    return u.pathname.startsWith('/api/') && u.pathname.endsWith(path)
  } catch {
    // Avoid matching page navigations when URL parsing fails.
    return false
  }
}

test.describe('Billing Settings — receipt preference save flow', () => {
  test('discard does not save; save persists and shows toast', async ({ page }) => {
    let putCalled = false
    let settingsGetCalled = false

    await page.addInitScript(() => {
      ;(window as any).__APEX_E2E_SESSION__ = {
        userId: 'usr-playwright',
        publisherId: 'pub-test-123',
        email: 'playwright@example.com',
        role: 'admin',
        permissions: ['billing:view', 'billing:manage'],
      }
    })

    await page.route('**/*', async (route) => {
      const req = route.request()
      const url = req.url()
      const method = req.method().toUpperCase()

      // CSRF bootstrap: allow the app's mock CSRF route (or fulfill a safe default)
      if (url.includes('/api/mock') && url.includes('endpoint=csrf') && method === 'GET') {
        return route.continue()
      }
      if (isApiPath(url, '/auth/csrf') && method === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          headers: { 'set-cookie': 'XSRF-TOKEN=faketoken; Path=/; HttpOnly' },
          body: JSON.stringify({ token: 'faketoken' }),
        })
      }

      // Session (used by Billing layout)
      if (isApiPath(url, '/auth/me') && method === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            success: true,
            data: {
              userId: 'usr-playwright',
              publisherId: 'pub-test-123',
              email: 'playwright@example.com',
              role: 'admin',
              permissions: ['billing:view', 'billing:manage'],
            },
          }),
        })
      }

      // Billing settings GET (page expects response.data to be the settings object)
      if (isApiPath(url, '/billing/settings') && method === 'GET') {
        settingsGetCalled = true
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            plan: {
              name: 'Growth',
              type: 'growth',
              platform_fee_rate: 0.1,
              platform_fee_label: 'Platform fee',
              revenue_band: '$10k-$50k',
              currency: 'USD',
              included_impressions: 500000,
              included_api_calls: 100000,
              included_data_transfer_gb: 50,
            },
            billing_email: 'billing@example.com',
            receipt_preferences: {
              send_receipts: true,
              send_invoices: true,
              send_usage_alerts: true,
            },
            stripe_customer_id: 'cus_test_123',
          }),
        })
      }

      // Feature flags (required by /settings/billing layout)
      if (isApiPath(url, '/meta/features') && method === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            success: true,
            data: {
              billing: true,
              transparency: true,
              migrationStudio: true,
            },
          }),
        })
      }

      // Billing policy GET (page expects response.data.data)
      if (isApiPath(url, '/billing/policy') && method === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            data: {
              summary: 'Billing policy',
              starterExperience: {
                paymentMethodRequired: false,
                revenueCapUsd: 1000,
                capabilities: ['basic'],
                messaging: 'Starter tier',
              },
              upgradePath: {
                triggers: ['Over cap'],
                acceptedPaymentMethods: [],
                autopay: {
                  defaultBehavior: 'Autopay enabled',
                  encouragement: 'Autopay recommended',
                  enterpriseOverride: 'Enterprise override',
                },
              },
              billingCycle: {
                cadence: 'monthly',
                notifications: [],
              },
              transparencyCommitments: {
                weDontTouchPayouts: "We don't touch payouts.",
                platformFeeOnly: 'Platform fee only.',
              },
            },
          }),
        })
      }

      // Receipt preferences PUT
      if (isApiPath(url, '/billing/settings/preferences') && method === 'PUT') {
        putCalled = true
        return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ success: true }) })
      }

      return route.continue()
    })

    await page.goto('/settings/billing/settings')
    await expect(page.getByRole('heading', { name: 'Billing Settings' })).toBeVisible()
    await expect.poll(() => settingsGetCalled, { timeout: 5000 }).toBeTruthy()

    // Toggle Monthly Invoices (aria-label is on the <label>)
    const invoiceToggle = page.getByLabel('Toggle monthly invoice emails')
    await invoiceToggle.click()

    // Discard changes should not save
    await page.getByRole('button', { name: 'Discard Changes' }).click()
    expect(putCalled).toBeFalsy()

    // Toggle again and save
    await invoiceToggle.click()
    const saveButton = page.getByRole('button', { name: 'Save Changes' })
    await expect(saveButton).toBeEnabled({ timeout: 5000 })
    await saveButton.click()

    await expect(page.getByText('Receipt preferences updated', { exact: false })).toBeVisible()
    await expect.poll(() => putCalled, { timeout: 5000 }).toBeTruthy()
  })
})
