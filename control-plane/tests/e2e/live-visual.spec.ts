// dogfood:skip:live/opt-in suite — requires a deployed environment (LIVE_E2E/CONSOLE_BASE_URL); enabled in the live lanes
import { test, expect } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'

async function liveLogin(page: any) {
  const username = process.env.LIVE_CONTROL_PLANE_USERNAME
  const password = process.env.LIVE_CONTROL_PLANE_PASSWORD

  if (!username || !password) {
    throw new Error('Missing LIVE_CONTROL_PLANE_USERNAME or LIVE_CONTROL_PLANE_PASSWORD')
  }

  await page.goto('/admin/login', { waitUntil: 'networkidle' })
  await page.waitForSelector('#username', { state: 'visible' })

  await page.fill('#username', username)
  await page.fill('#password', password)

  await page.evaluate(async () => {
    const username = (document.getElementById('username') as HTMLInputElement).value
    const password = (document.getElementById('password') as HTMLInputElement).value

    let deviceId = ''
    try {
      const existing = localStorage.getItem('controlPlaneDeviceId')
      if (existing && existing.length >= 16) {
        deviceId = existing
      } else {
        const bytes = new Uint8Array(32)
        if (window.crypto && window.crypto.getRandomValues) {
          window.crypto.getRandomValues(bytes)
        }
        const arr = Array.from(bytes)
        const bin = arr.map((b) => String.fromCharCode(b)).join('')
        deviceId = btoa(bin).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/g, '')
        localStorage.setItem('controlPlaneDeviceId', deviceId)
      }
    } catch (e) {
      deviceId = 'test-device-id'
    }

    const deviceLabel = navigator.userAgent.slice(0, 120)
    const resp = await fetch('/api/v1/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password, deviceId, deviceLabel }),
    })

    if (!resp.ok) {
      throw new Error(`Login failed: ${resp.status}`)
    }

    const data = await resp.json()
    if (data.token) {
      localStorage.setItem('controlPlaneToken', data.token)
    }

    window.location.href = '/admin'
  })

  await page.waitForURL('**/admin**', { timeout: 30_000 })
  await expect(page.locator('.brand')).toContainText('APEX CONTROL PLANE')
}

test.describe('Control Plane LIVE — visual verification', () => {
  test.describe.configure({ timeout: 120_000 })

  test('capture telemetry + data explorer views', async ({ page }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live runs')

    await liveLogin(page)

    await page.locator('.header .nav-item[data-view="telemetry"]').click({ timeout: 10_000 })
    await expect(page.locator('.header .nav')).toBeVisible({ timeout: 10_000 })
    await page.waitForTimeout(1500)
    try {
      await page.screenshot({ path: 'test-results/visual-telemetry.png' })
    } catch {
      await page.screenshot({ path: 'test-results/visual-telemetry.png', fullPage: false })
    }

    const telemetryDebug = await page.evaluate(() => {
      const header = document.querySelector('.header') as HTMLElement | null
      const nav = document.querySelector('.header .nav') as HTMLElement | null
      const navItem = document.querySelector('.nav-item[data-view="telemetry"]') as HTMLElement | null
      return {
        url: window.location.href,
        headerExists: Boolean(header),
        navExists: Boolean(nav),
        navDisplay: nav ? getComputedStyle(nav).display : null,
        navVisibility: nav ? getComputedStyle(nav).visibility : null,
        navOpacity: nav ? getComputedStyle(nav).opacity : null,
        navItemExists: Boolean(navItem),
        bodyOverflow: getComputedStyle(document.body).overflow,
        contentOverflow: getComputedStyle(document.querySelector('.content') as HTMLElement).overflowY,
      }
    })
    const telemetryDebugPath = path.join('test-results', 'visual-telemetry-debug.json')
    fs.writeFileSync(telemetryDebugPath, JSON.stringify(telemetryDebug, null, 2))

    await page.locator('.header .nav-item[data-view="data-explorer"]').click({ timeout: 10_000 })
    await expect(page.locator('#view-data-explorer')).toBeVisible({ timeout: 10_000 })
    await page.waitForTimeout(2000)
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight))
    await page.waitForTimeout(1000)
    try {
      await page.screenshot({ path: 'test-results/visual-data-explorer-bottom.png' })
    } catch {
      await page.screenshot({ path: 'test-results/visual-data-explorer-bottom.png', fullPage: false })
    }

    const clippingDebug = await page.evaluate(() => {
      const invoicesTable = document.querySelector('#invoices-table table')
      const usersTable = document.querySelector('#users-table table')
      const invoiceRows = invoicesTable?.querySelectorAll('tbody tr') || []
      const userRows = usersTable?.querySelectorAll('tbody tr') || []
      const lastInvoiceRow = invoiceRows.length ? (invoiceRows[invoiceRows.length - 1] as HTMLElement) : null
      const lastUserRow = userRows.length ? (userRows[userRows.length - 1] as HTMLElement) : null
      const rect = (lastInvoiceRow || lastUserRow)?.getBoundingClientRect()
      return {
        viewportHeight: window.innerHeight,
        invoiceRowCount: invoiceRows.length,
        userRowCount: userRows.length,
        lastRowRect: rect
          ? { top: rect.top, bottom: rect.bottom, height: rect.height }
          : null,
      }
    })
    const clippingDebugPath = path.join('test-results', 'visual-data-explorer-debug.json')
    fs.writeFileSync(clippingDebugPath, JSON.stringify(clippingDebug, null, 2))
  })
})
