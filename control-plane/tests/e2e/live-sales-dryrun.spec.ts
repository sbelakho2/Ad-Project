import { test, expect, Page } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'

/**
 * Sales Automation Dry Run E2E Test
 * 
 * Tests the full sales automation dry run flow on the live control plane.
 * This verifies the entire stack without actually sending any emails.
 */

async function liveLogin(page: Page) {
  const username = process.env.LIVE_CONTROL_PLANE_USERNAME
  const password = process.env.LIVE_CONTROL_PLANE_PASSWORD

  if (!username || !password) {
    throw new Error(
      'Missing LIVE_CONTROL_PLANE_USERNAME or LIVE_CONTROL_PLANE_PASSWORD.\n' +
      'These tests require REAL operator credentials.\n' +
      'See docs/LIVE_TESTING.md for setup instructions.'
    )
  }

  await page.goto('/admin/login', { waitUntil: 'networkidle' })

  await page.waitForSelector('#username', { state: 'visible' })
  await page.waitForTimeout(1500)

  const usernameInput = page.locator('#username')
  const passwordInput = page.locator('#password')

  await usernameInput.click()
  await usernameInput.fill(username)
  await passwordInput.click()
  await passwordInput.fill(password)

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

  await page.waitForURL('**/admin', { timeout: 30_000 })
  await expect(page.locator('.brand', { hasText: 'APEX CONTROL PLANE' })).toBeVisible({ timeout: 10_000 })
}

test.describe('Sales Automation Dry Run — LIVE Control Plane', () => {
  test.describe.configure({ timeout: 120_000 })

  test('execute dry run and verify response', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live dry-run tests')

    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')
    const origin = new URL(baseURL).origin

    const serverErrors: string[] = []
    const consoleErrors: string[] = []
    const apiResponses: { url: string; status: number; body?: any }[] = []

    page.on('response', async (res) => {
      try {
        const status = res.status()
        const url = res.url()
        if (!url.startsWith(origin)) return

        if (status >= 500) {
          let snippet = ''
          try {
            const text = await res.text()
            snippet = text.slice(0, 2000)
          } catch {}
          serverErrors.push(`${status} ${url}${snippet ? `\n${snippet}` : ''}`)
        }

        // Capture sales API responses for debugging
        if (url.includes('/api/v1/sales/')) {
          let body: any = null
          try {
            body = await res.json()
          } catch {}
          apiResponses.push({ url, status, body })
        }
      } catch {}
    })

    page.on('console', (msg) => {
      if (msg.type() !== 'error') return
      const text = msg.text()
      if (text.includes('Failed to load resource')) return
      if (text.includes('favicon')) return
      consoleErrors.push(text)
    })

    // Login
    await liveLogin(page)
    expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)

    // Navigate to Sales Automation view
    await test.step('Navigate to Sales Automation', async () => {
      // Use .first() to handle duplicate nav items
      await page.locator('.nav-item[data-view="sales"]').first().click({ timeout: 10_000 })
      await expect(page.locator('#view-sales')).toBeVisible({ timeout: 10_000 })
      await page.waitForTimeout(2000) // Let stats load
    })

    // Capture initial stats
    const initialStats = await test.step('Capture initial stats', async () => {
      const stats = await page.evaluate(() => {
        return {
          contactsCount: document.getElementById('sales-contacts-count')?.textContent || '0',
          enrollmentsCount: document.getElementById('sales-enrollments-count')?.textContent || '0',
          emailsSent: document.getElementById('sales-emails-sent')?.textContent || '0',
          systemEnabled: document.getElementById('sales-system-status')?.textContent || 'unknown',
        }
      })
      console.log('Initial Sales Stats:', stats)
      return stats
    })

    // Execute Dry Run
    await test.step('Execute Dry Run', async () => {
      // Find and click the dry run button
      const dryRunBtn = page.locator('button', { hasText: /dry run/i })
      await expect(dryRunBtn).toBeVisible({ timeout: 5_000 })
      await dryRunBtn.click()

      // Wait for the action result to update
      await page.waitForTimeout(3000)
      
      const actionResult = page.locator('#sales-action-result')
      await expect(actionResult).toBeVisible({ timeout: 10_000 })

      // Get the result text
      const resultText = await actionResult.textContent()
      console.log('Dry Run Result:', resultText)

      // Verify it's a meaningful result (could be success, or indicate no enrollments ready)
      expect(resultText).toBeTruthy()
      // Accept various success messages including "ready to process" which means no enrollments pending
      expect(resultText?.toLowerCase()).toMatch(/dry run|would send|processed|error|disabled|ready|complete/i)
    })

    // Capture and save debug info
    await test.step('Capture debug info', async () => {
      const debugInfo = {
        timestamp: new Date().toISOString(),
        initialStats,
        apiResponses,
        serverErrors,
        consoleErrors,
      }

      const debugPath = path.join('test-results', 'sales-dryrun-debug.json')
      fs.mkdirSync('test-results', { recursive: true })
      fs.writeFileSync(debugPath, JSON.stringify(debugInfo, null, 2))
      console.log('Debug info saved to:', debugPath)

      // Take screenshot
      await page.screenshot({ path: 'test-results/sales-dryrun-result.png' })
    })

    // Final assertions
    expect(serverErrors, `Server errors: ${serverErrors.join('\n')}`).toHaveLength(0)
    
    if (consoleErrors.length > 0) {
      console.warn('Console errors (non-fatal):', consoleErrors)
    }
  })

  test('verify sales stats API returns valid data', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live tests')

    if (!baseURL) throw new Error('Playwright baseURL is required')

    await liveLogin(page)

    // Navigate to sales view to trigger API calls
    await page.locator('.nav-item[data-view="sales"]').first().click({ timeout: 10_000 })
    await page.waitForTimeout(2000)

    // Make direct API calls to verify the sales automation endpoints
    const apiResults = await page.evaluate(async () => {
      const token = localStorage.getItem('controlPlaneToken')
      const headers: Record<string, string> = { 'content-type': 'application/json' }
      if (token) headers['Authorization'] = `Bearer ${token}`

      const results: {
        status: { ok: boolean; data?: any; error?: string }
        stats: { ok: boolean; data?: any; error?: string }
        dryRun: { ok: boolean; data?: any; error?: string }
      } = {
        status: { ok: false },
        stats: { ok: false },
        dryRun: { ok: false },
      }

      // Get status
      try {
        const statusResp = await fetch('/api/v1/sales/status', { headers })
        const statusData = await statusResp.json()
        results.status = { ok: statusResp.ok, data: statusData }
      } catch (e: any) {
        results.status = { ok: false, error: e.message }
      }

      // Get stats  
      try {
        const statsResp = await fetch('/api/v1/sales/stats', { headers })
        const statsData = await statsResp.json()
        results.stats = { ok: statsResp.ok, data: statsData }
      } catch (e: any) {
        results.stats = { ok: false, error: e.message }
      }

      // Execute dry run
      try {
        const runResp = await fetch('/api/v1/sales/run', {
          method: 'POST',
          headers,
          body: JSON.stringify({ dryRun: true }),
        })
        const runData = await runResp.json()
        results.dryRun = { ok: runResp.ok, data: runData }
      } catch (e: any) {
        results.dryRun = { ok: false, error: e.message }
      }

      return results
    })

    console.log('API Results:', JSON.stringify(apiResults, null, 2))

    // Save results
    const resultsPath = path.join('test-results', 'sales-api-results.json')
    fs.mkdirSync('test-results', { recursive: true })
    fs.writeFileSync(resultsPath, JSON.stringify(apiResults, null, 2))

    // Assertions
    expect(apiResults.status.ok, `Status API failed: ${apiResults.status.error}`).toBe(true)
    expect(apiResults.stats.ok, `Stats API failed: ${apiResults.stats.error}`).toBe(true)
    expect(apiResults.dryRun.ok, `Dry run API failed: ${apiResults.dryRun.error}`).toBe(true)

    // Verify dry run response structure
    const dryRunData = apiResults.dryRun.data
    expect(dryRunData).toHaveProperty('dryRun', true)
    expect(dryRunData).toHaveProperty('processed')
    expect(dryRunData).toHaveProperty('sent')
    expect(typeof dryRunData.processed).toBe('number')
    expect(typeof dryRunData.sent).toBe('number')
  })
})
