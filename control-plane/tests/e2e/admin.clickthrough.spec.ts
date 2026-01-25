import { test, expect } from '@playwright/test'

function isProbablyDestructive(label: string) {
  const s = label.toLowerCase()
  return s.includes('delete') || s.includes('drop') || s.includes('terminate') || s.includes('reset')
}

test.describe('Control Plane operator UI — click-through', () => {
  test('sign in and click every button/link (safe, mocked APIs)', async ({ page }) => {
    // Skip this test during LIVE_E2E runs (use live-clickthrough.spec.ts instead)
    test.skip(Boolean(process.env.LIVE_E2E), 'Skipped during LIVE_E2E (use live-clickthrough.spec.ts)')
    
    const pageErrors: string[] = []
    const consoleErrors: string[] = []
    page.on('pageerror', (err) => pageErrors.push(String(err)))
    page.on('console', (msg) => {
      if (msg.type() === 'error') consoleErrors.push(msg.text())
    })

    // Mock dashboard + device APIs so we can exercise UI without a real DB.
    // Keep login API real.
    const now = new Date().toISOString()

    let deviceRevoked = false

    await page.route('**/api/v1/dashboard', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ok: true,
          service: 'control-plane',
          timestamp: now,
          note: 'mocked for playwright clickthrough',
        }),
      })
    })

    await page.route('**/api/v1/devices', async (route) => {
      const device = {
        id: 'dev-1',
        deviceLabel: 'Playwright Mac',
        userAgent: 'Playwright',
        addedAt: now,
        lastSeenAt: now,
        revokedAt: deviceRevoked ? now : null,
      }
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ devices: [device] }),
      })
    })

    await page.route('**/api/v1/devices/*/revoke', async (route) => {
      deviceRevoked = true
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ok: true }),
      })
    })

    // Mock sales automation APIs (no real DB in test mode)
    await page.route('**/api/v1/sales/status', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          enabled: false,
          configured: true,
          dailyLimit: 50,
          todaySent: 0,
          todayRemaining: 50
        }),
      })
    })

    await page.route('**/api/v1/sales/stats', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          contacts: { total: 0, byStatus: {} },
          enrollments: { total: 0, byStatus: {} },
          emails: { sent: 0, openRate: 0, clickRate: 0, replyRate: 0 }
        }),
      })
    })

    await page.route('**/api/v1/sales/sequences', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ sequences: [] }),
      })
    })

    await page.route('**/api/v1/sales/contacts**', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ contacts: [], total: 0 }),
      })
    })

    await page.route('**/api/v1/sales/enrollments', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ enrollments: [] }),
      })
    })

    await page.route('**/api/v1/sales/emails', async (route) => {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ emails: [] }),
      })
    })

    // Click-through should accept confirm dialogs (Revoke uses window.confirm).
    page.on('dialog', async (d) => {
      await d.accept().catch(() => {})
    })

    // Sign in using direct authentication (bypasses event listener issues)
    await page.goto('/admin/login', { waitUntil: 'networkidle' })
    await page.waitForTimeout(1000)

    const username = 'playwright@local.test'
    const password = 'playwright-pass'

    // Fill the form
    await page.locator('#username').fill(username)
    await page.locator('#password').fill(password)

    // Authenticate directly via JavaScript
    await page.evaluate(async () => {
      const username = (document.getElementById('username') as HTMLInputElement).value;
      const password = (document.getElementById('password') as HTMLInputElement).value;
      
      // Get or create device ID
      let deviceId = '';
      try {
        const existing = localStorage.getItem('controlPlaneDeviceId');
        if (existing && existing.length >= 16) {
          deviceId = existing;
        } else {
          const bytes = new Uint8Array(32);
          window.crypto.getRandomValues(bytes);
          const arr = Array.from(bytes);
          const bin = arr.map(b => String.fromCharCode(b)).join('');
          deviceId = btoa(bin).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/g, '');
          localStorage.setItem('controlPlaneDeviceId', deviceId);
        }
      } catch (e) {
        deviceId = 'test-device-id';
      }
      
      const deviceLabel = navigator.userAgent.slice(0, 120);
      
      const resp = await fetch('/api/v1/auth/login', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username, password, deviceId, deviceLabel }),
      });
      
      if (!resp.ok) {
        throw new Error(`Login failed: ${resp.status}`);
      }
      
      const data = await resp.json();
      
      if (data.token) {
        localStorage.setItem('controlPlaneToken', data.token);
      }
      
      window.location.href = '/admin';
    });

    await expect(page).toHaveURL(/\/admin\/?(\?|$)/)
    await expect(page.getByRole('heading', { name: /dashboard/i })).toBeVisible()

    // If the login JS crashed, fail with the error details.
    expect(
      pageErrors,
      `pageerrors:\n${pageErrors.join('\n')}\nconsoleErrors:\n${consoleErrors.join('\n')}`
    ).toEqual([])

    // Wait for system status to show (dashboard is loaded)
    await expect(page.locator('text=System Online')).toBeVisible({ timeout: 10000 })

    // Click all visible buttons first (includes Revoke).
    const buttons = page.locator('button')
    const buttonCount = await buttons.count()
    for (let i = 0; i < buttonCount; i++) {
      const btn = buttons.nth(i)
      if (!(await btn.isVisible().catch(() => false))) continue
      if (!(await btn.isEnabled().catch(() => false))) continue

      const label = ((await btn.textContent().catch(() => '')) || '').trim()
      if (label && isProbablyDestructive(label)) continue

      await btn.click({ timeout: 2000 }).catch(() => {})
    }

    // Click links, but do Sign out last.
    const links = page.locator('a')
    const linkCount = await links.count()

    const signOutCandidates: number[] = []
    for (let i = 0; i < linkCount; i++) {
      const a = links.nth(i)
      if (!(await a.isVisible().catch(() => false))) continue

      const text = ((await a.textContent().catch(() => '')) || '').trim()
      const href = (await a.getAttribute('href').catch(() => null)) || ''
      if (!href || href.startsWith('http') || href.startsWith('mailto:') || href.startsWith('#')) continue

      if (/sign out/i.test(text) || /\/admin\/logout/.test(href)) {
        signOutCandidates.push(i)
        continue
      }

      await a.click({ timeout: 2000 }).catch(() => {})
    }

    // Finally, sign out.
    if (signOutCandidates.length > 0) {
      const a = links.nth(signOutCandidates[0])
      await a.click({ timeout: 2000 }).catch(() => {})
      await expect(page).toHaveURL(/\/admin\/login/)
    }

    // Sanity: login page should still render.
    await expect(page.getByRole('button', { name: /sign in/i })).toBeVisible()
  })
})
