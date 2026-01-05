import { test, expect, type Page } from '@playwright/test'
import fs from 'node:fs'

type Snapshot = {
  url: string
  title: string
  htmlLen: number
  textSnippet: string
  hasSkeleton: boolean
  hasNav: boolean
  hasHeadingFraud: boolean
  hasHeadingTransparency: boolean
}

async function snapshot(page: Page): Promise<Snapshot> {
  const url = page.url()
  const title = await page.title().catch(() => '(no title)')
  const html = await page.content()
  const text = await page.evaluate(() => (document.body?.innerText || '').replace(/\s+/g, ' ').trim())

  const hasSkeleton = await page.locator('.animate-pulse').first().isVisible().catch(() => false)
  const hasNav = await page.locator('nav').first().isVisible().catch(() => false)
  const hasHeadingFraud = await page.getByRole('heading', { name: /fraud/i }).first().isVisible().catch(() => false)
  const hasHeadingTransparency = await page.getByRole('heading', { name: /transparency/i }).first().isVisible().catch(() => false)

  return {
    url,
    title,
    htmlLen: html.length,
    textSnippet: text.slice(0, 400),
    hasSkeleton,
    hasNav,
    hasHeadingFraud,
    hasHeadingTransparency,
  }
}

type LiveAuthResult =
  | { success: true; token: string }
  | { success: false; error: string }

async function liveLoginViaApi(page: Page) {
  const email = process.env.LIVE_CONSOLE_EMAIL
  const password = process.env.LIVE_CONSOLE_PASSWORD
  const twofa = process.env.LIVE_CONSOLE_2FA_CODE

  if (!email || !password) {
    throw new Error('Missing LIVE_CONSOLE_EMAIL or LIVE_CONSOLE_PASSWORD')
  }

  await page.goto('https://console.apexmediation.ee/login', { waitUntil: 'domcontentloaded', timeout: 30_000 })
  const origin = new URL(page.url()).origin

  const authResult = await page.evaluate<LiveAuthResult, { email: string; password: string; twofa?: string }>(
    async ({ email, password, twofa }: { email: string; password: string; twofa?: string }) => {
      return new Promise((resolve) => {
        try {
          const xhr = new XMLHttpRequest()
          xhr.open('POST', 'https://api.apexmediation.ee/api/v1/auth/login', true)
          xhr.setRequestHeader('Content-Type', 'application/json')
          xhr.withCredentials = true

          xhr.onload = function () {
            try {
              if (xhr.status !== 200) {
                resolve({ success: false, error: `Login failed: ${xhr.status} ${xhr.responseText}` })
                return
              }

              const data = JSON.parse(xhr.responseText)
              const tempToken = data?.data?.tempToken
              const twofaRequired = Boolean(data?.data?.twofaRequired)

              if (twofaRequired) {
                if (!twofa) {
                  resolve({ success: false, error: '2FA is required for this user. Set LIVE_CONSOLE_2FA_CODE and re-run.' })
                  return
                }

                const xhr2fa = new XMLHttpRequest()
                xhr2fa.open('POST', 'https://api.apexmediation.ee/api/v1/auth/login/2fa', true)
                xhr2fa.setRequestHeader('Content-Type', 'application/json')
                xhr2fa.withCredentials = true
                xhr2fa.onload = function () {
                  try {
                    if (xhr2fa.status !== 200) {
                      resolve({ success: false, error: `2FA login failed: ${xhr2fa.status} ${xhr2fa.responseText}` })
                      return
                    }
                    const twofaData = JSON.parse(xhr2fa.responseText)
                    const token = twofaData.data?.token || twofaData.token
                    if (!token) {
                      resolve({ success: false, error: 'No token in 2FA response' })
                      return
                    }
                    resolve({ success: true, token })
                  } catch (err) {
                    resolve({ success: false, error: `2FA response parse error: ${err}` })
                  }
                }
                xhr2fa.onerror = function () {
                  resolve({ success: false, error: '2FA request failed' })
                }
                xhr2fa.send(JSON.stringify({ tempToken, code: twofa }))
                return
              }

              const token = data.data?.token || data.token
              if (!token) {
                resolve({ success: false, error: 'No token in response' })
                return
              }
              resolve({ success: true, token })
            } catch (err) {
              resolve({ success: false, error: `Login parse error: ${err}` })
            }
          }

          xhr.onerror = function () {
            resolve({ success: false, error: 'Login request failed' })
          }

          xhr.send(JSON.stringify({ email, password }))
        } catch (error) {
          resolve({ success: false, error: String(error) })
        }
      })
    },
    { email, password, twofa }
  )

  if (!authResult.success) {
    throw new Error(`Login failed: ${authResult.error || 'unknown'}`)
  }

  const domain = new URL(origin).hostname
  const isSecure = origin.startsWith('https')
  await page.context().addCookies([
    {
      name: 'access_token',
      value: authResult.token,
      domain,
      path: '/',
      httpOnly: true,
      secure: isSecure,
      sameSite: 'Lax',
    },
    {
      name: 'next-auth.session-token',
      value: authResult.token,
      domain,
      path: '/',
      httpOnly: false,
      secure: isSecure,
      sameSite: 'Lax',
    },
  ])

  if (isSecure) {
    await page.context().addCookies([
      {
        name: '__Secure-next-auth.session-token',
        value: authResult.token,
        domain,
        path: '/',
        httpOnly: false,
        secure: true,
        sameSite: 'Lax',
      },
    ])
  }
}

test('live hydration debug: fraud + transparency', async ({ page }) => {
  test.setTimeout(90_000)
  fs.mkdirSync('test-results', { recursive: true })

  const consoleMessages: Array<{ type: string; text: string }> = []
  const pageErrors: string[] = []
  const failedRequests: string[] = []
  const badResponses: string[] = []
  const rscResponses: string[] = []
  const fraudChunkResponses: string[] = []

  page.on('console', (msg) => {
    consoleMessages.push({ type: msg.type(), text: msg.text() })
  })
  page.on('pageerror', (err) => pageErrors.push(err.message))
  page.on('requestfailed', (req) => {
    const f = req.failure()?.errorText || 'unknown'
    failedRequests.push(`${req.method()} ${req.url()} :: ${f}`)
  })
  page.on('response', (resp) => {
    const s = resp.status()
    const url = resp.url()
    if (url.includes('_rsc=')) {
      rscResponses.push(`${s} ${url}`)
    }
    if (url.includes('/_next/static/chunks/app/fraud/')) {
      fraudChunkResponses.push(`${s} ${url}`)
    }
    if (s >= 400) badResponses.push(`${s} ${url}`)
  })

  await liveLoginViaApi(page)

  // Hit dashboard first (control)
  await page.goto('https://console.apexmediation.ee/dashboard', { waitUntil: 'load', timeout: 30000 })
  await page.waitForTimeout(3000)
  const dash = await snapshot(page)

  // Analytics
  await page.goto('https://console.apexmediation.ee/analytics', { waitUntil: 'load', timeout: 30000 })
  await page.waitForTimeout(6000)
  const analytics = await snapshot(page)

  // Adapters
  await page.goto('https://console.apexmediation.ee/adapters', { waitUntil: 'load', timeout: 30000 })
  await page.waitForTimeout(6000)
  const adapters = await snapshot(page)

  // Fraud
  await page.goto('https://console.apexmediation.ee/fraud', { waitUntil: 'load', timeout: 30000 })
  await page.waitForTimeout(12000)
  const fraud = await snapshot(page)

  // Transparency auctions
  await page.goto('https://console.apexmediation.ee/transparency/auctions', { waitUntil: 'load', timeout: 30000 })
  await page.waitForTimeout(8000)
  const auctions = await snapshot(page)

  // Transparency summary
  await page.goto('https://console.apexmediation.ee/transparency/summary', { waitUntil: 'load', timeout: 30000 })
  await page.waitForTimeout(6000)
  const trans = await snapshot(page)

  const report = {
    at: new Date().toISOString(),
    dashboard: dash,
    analytics,
    adapters,
    fraud,
    transparencyAuctions: auctions,
    transparencySummary: trans,
    pageErrors,
    consoleMessages,
    requestFailures: failedRequests,
    badResponses,
    rscResponses,
    fraudChunkResponses,
  }
  fs.writeFileSync('test-results/live-debug-hydration.json', JSON.stringify(report, null, 2), 'utf-8')

  console.log('Wrote test-results/live-debug-hydration.json')
  console.log('badResponses:', badResponses.length, 'requestFailures:', failedRequests.length, 'pageErrors:', pageErrors.length)

  // No asserts: diagnostic-only
  expect(true).toBeTruthy()
})
