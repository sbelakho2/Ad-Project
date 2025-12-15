import { test, expect } from '@playwright/test'

// Smoke test for Payouts settings with confirmation modal flow.
// This test stubs network calls to avoid backend dependencies and focuses on UI logic:
// - Navigate to /settings/payouts
// - Load current settings via GET /settings/payout
// - Edit a field and click Save → confirmation modal appears
// - Cancel: ensure no PUT request is sent
// - Confirm & Save: PUT /settings/payout is sent and success message shows

// Helpers to recognize endpoints regardless of origin/prefix
function isPath(url: string, path: string) {
  try {
    const u = new URL(url)
    return u.pathname.endsWith(path)
  } catch {
    return url.endsWith(path)
  }
}

test.describe('Payouts Settings — confirmation modal flow', () => {
  test('open modal, cancel (no PUT), then confirm & save (PUT)', async ({ page }) => {
    // Track if PUT was called
    let putCalled = false

    // Catch-all routing: stub CSRF, GET/PUT settings regardless of origin/prefix
    let getCalled = false
    await page.route('**/*', async (route) => {
      const req = route.request()
      const url = req.url()
      const method = req.method().toUpperCase()

      // CSRF bootstrap
      if (isPath(url, '/auth/csrf') && method === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          headers: { 'set-cookie': 'XSRF-TOKEN=faketoken; Path=/; HttpOnly' },
          body: JSON.stringify({ token: 'faketoken' }),
        })
      }

      // Payout settings GET
      if (isPath(url, '/settings/payout') && method === 'GET') {
        getCalled = true
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            success: true,
            data: {
              method: 'wire',
              accountName: 'Acme Corp',
              accountReference: '**** 1234',
              currency: 'USD',
              threshold: 100,
              schedule: 'monthly',
              autoPayout: false,
              backupMethod: null,
              updatedAt: new Date().toISOString(),
            },
          }),
        })
      }

      // Payout settings PUT
      if (isPath(url, '/settings/payout') && method === 'PUT') {
        putCalled = true
        const payload = JSON.parse(req.postData() || '{}')
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ success: true, data: payload }),
        })
      }

      return route.continue()
    })

    await page.goto('/settings/payouts')

    // Ensure the page loaded
    await expect(page.getByRole('heading', { name: 'Payouts & Billing' })).toBeVisible()

    // Wait for initial data load to populate the form (from stubbed GET)
    const accountName = page.getByLabel('Account Name')
    await expect(accountName).toBeVisible()
    // Ensure our GET stub was actually called and applied
    await expect.poll(() => getCalled, { timeout: 5000 }).toBeTruthy()
    await expect(accountName).toHaveValue('Acme Corp')

    // Edit a field (Account Name)
    await accountName.clear()
    await accountName.fill('Acme Finance LLC')

    // Wait until save is enabled (query finished and no pending mutation)
    const saveButton = page.getByRole('button', { name: 'Save Settings' })
    await expect(saveButton).toBeEnabled({ timeout: 5000 })

    // Click Save → should show confirmation modal
    await saveButton.click()
    const dialog = page.getByRole('dialog', { name: /Confirm sensitive account changes/i })
    await expect(dialog).toBeVisible()

    // Cancel → modal closes; ensure no PUT called
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog).toBeHidden()
    expect(putCalled).toBeFalsy()

    // Click Save again → modal → type CONFIRM → Confirm & Save
    await page.getByRole('button', { name: 'Save Settings' }).click()
    const dialog2 = page.getByRole('dialog', { name: /Confirm sensitive account changes/i })
    await expect(dialog2).toBeVisible()
    
    // The confirm button is disabled until user types "CONFIRM"
    const confirmInput = dialog2.locator('input#confirm-input')
    await confirmInput.fill('CONFIRM')
    await dialog2.getByRole('button', { name: /Confirm & Save/i }).click()

    // Success message appears and PUT was called once
    await expect(page.getByText('Payout settings updated successfully.', { exact: false })).toBeVisible()
    expect(putCalled).toBeTruthy()
  })
})
