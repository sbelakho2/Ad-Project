import { test, expect, Page } from '@playwright/test'
import fs from 'node:fs'

type ClickOptions = {
  maxClicks?: number
  timeBudgetMs?: number
  rounds?: number
}

type InflightRequestInfo = {
  startedAt: number
  url: string
  method: string
}

function isProbablyDestructive(label: string) {
  const s = label.toLowerCase()
  return (
    s.includes('delete') ||
    s.includes('remove') ||
    s.includes('revoke') ||
    s.includes('disable') ||
    s.includes('terminate') ||
    s.includes('drop') ||
    s.includes('reset') ||
    s.includes('destroy') ||
    s.includes('wipe') ||
    s.includes('sign out') ||
    s.includes('log out') ||
    s === 'logout'
  )
}

function isProbablySideEffectful(label: string) {
  const s = label.toLowerCase()
  return (
    s.includes('save') ||
    s.includes('create') ||
    s.includes('add ') ||
    s.startsWith('add') ||
    s.includes('invite') ||
    s.includes('send') ||
    s.includes('submit') ||
    s.includes('purchase') ||
    s.includes('upgrade') ||
    s.includes('pay') ||
    s.includes('generate') ||
    s.includes('enable') ||
    s.includes('rotate') ||
    s.includes('regenerate') ||
    s.includes('enroll')
  )
}

function isSafeSubmit(label: string) {
  const s = label.toLowerCase()
  return (
    s.includes('search') ||
    s.includes('filter') ||
    s.includes('apply') ||
    s.includes('refresh') ||
    s.includes('reload') ||
    s.includes('test') ||
    s.includes('validate') ||
    s.includes('preview')
  )
}

async function closeAnyModal(page: Page) {
  if (page.isClosed()) return
  await page.getByRole('button', { name: /cancel|close|dismiss|not now|back/i }).first().click({ timeout: 750 }).catch(() => {})
  await page.keyboard.press('Escape').catch(() => {})
}

async function clickVisibleButtons(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 40
  const timeBudgetMs = options?.timeBudgetMs ?? 7_500
  const rounds = options?.rounds ?? 2
  const startedAt = Date.now()

  page.on('dialog', async (d) => {
    await d.dismiss().catch(() => {})
  })

  await page.waitForLoadState('domcontentloaded')

  const buttons = page.locator(
    [
      'button:not([disabled])',
      '[role="button"]:not([aria-disabled="true"])',
      '[role="menuitem"]:not([aria-disabled="true"])',
    ].join(', ')
  )

  let clicks = 0
  for (let round = 0; round < rounds && clicks < maxClicks; round++) {
    await page.waitForLoadState('domcontentloaded')
    await page.waitForTimeout(100).catch(() => {})

    // Scroll to reveal more content/buttons.
    await page.evaluate(() => window.scrollTo(0, 0)).catch(() => {})
    await page.waitForTimeout(50).catch(() => {})
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight)).catch(() => {})
    await page.waitForTimeout(100).catch(() => {})
    await page.evaluate(() => window.scrollTo(0, 0)).catch(() => {})

    const count = await buttons.count().catch(() => 0)

    for (let i = 0; i < count && clicks < maxClicks; i++) {
      if (Date.now() - startedAt > timeBudgetMs) break
      if (page.isClosed()) break

      const btn = buttons.nth(i)
      if (!(await btn.isVisible().catch(() => false))) continue

      const text = (await btn.textContent().catch(() => ''))?.trim()
      const ariaLabel = (await btn.getAttribute('aria-label').catch(() => null))
      const label = (text || ariaLabel || '').trim()

      // Skip unlabeled/icon-only controls. They tend to be menus or destructive
      // actions and aren’t useful for a broad smoke click-through.
      if (!label) continue

      if (label && isProbablyDestructive(label)) continue

      if (label && isProbablySideEffectful(label)) continue

      const typeAttr = (await btn.getAttribute('type').catch(() => null))
      if (typeAttr === 'submit') {
        // Allow only obviously safe submits (search/filter/apply).
        if (!label || !isSafeSubmit(label)) continue
      }

      await btn.scrollIntoViewIfNeeded().catch(() => {})
      await btn.click({ timeout: 750 }).catch(() => {})

      clicks++
      if (clicks % 6 === 0) await closeAnyModal(page)
      await page.waitForTimeout(50).catch(() => {})
    }
  }
}

async function startServerErrorCapture(page: Page, origin: string) {
  const serverErrors: string[] = []

  page.on('response', async (res) => {
    try {
      const status = res.status()
      if (status < 500) return
      const url = res.url()
      if (!url.startsWith(origin)) return

      // Production may intentionally disable some endpoints behind feature flags.
      // Treat these as expected, non-fatal states for click-through.
      if (status === 503 && url.includes('/api/v1/transparency/')) {
        try {
          const text = await res.text()
          if (text.includes('Transparency API is disabled')) return
        } catch {
          // ignore
        }
      }

      let snippet = ''
      try {
        const text = await res.text()
        snippet = text.slice(0, 2000)
      } catch {
        // ignore non-text bodies
      }

      serverErrors.push(`${status} ${url}${snippet ? `\n${snippet}` : ''}`)
    } catch {
      // best-effort capture only
    }
  })

  return serverErrors
}

async function gotoWithRetry(page: Page, route: string) {
  const maxAttempts = 4
  let lastError: unknown

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      await page.goto(route, { waitUntil: 'domcontentloaded', timeout: 60_000 })
      return
    } catch (e) {
      lastError = e
      const msg = String(e)
      const looksTransient =
        msg.includes('net::ERR_ADDRESS_UNREACHABLE') ||
        msg.includes('net::ERR_NAME_NOT_RESOLVED') ||
        msg.includes('net::ERR_CONNECTION_CLOSED') ||
        msg.includes('net::ERR_CONNECTION_RESET') ||
        msg.includes('net::ERR_CONNECTION_REFUSED') ||
        msg.includes('net::ERR_TIMED_OUT')

      if (!looksTransient) throw e

      // Exponential-ish backoff: 0.75s, 1.5s, 3s
      const waitMs = Math.min(3_000, 750 * Math.pow(2, attempt - 1))
      await page.waitForTimeout(waitMs)
    }
  }

  throw new Error(`Failed to navigate to ${route} after ${maxAttempts} attempts: ${String(lastError)}`)
}

function startInflightRequestCapture(page: Page, origin: string) {
  const inflight = new Map<any, InflightRequestInfo>()

  const track = (req: any) => {
    try {
      const url = req.url?.() ?? ''
      if (!url.startsWith(origin)) return
      inflight.set(req, {
        startedAt: Date.now(),
        url,
        method: req.method?.() ?? 'GET',
      })
    } catch {
      // best-effort
    }
  }

  const untrack = (req: any) => {
    inflight.delete(req)
  }

  page.on('request', track)
  page.on('requestfinished', untrack)
  page.on('requestfailed', untrack)

  const snapshot = () => {
    const now = Date.now()
    return Array.from(inflight.values())
      .map((r) => ({ ...r, ageMs: now - r.startedAt }))
      .sort((a, b) => b.ageMs - a.ageMs)
  }

  return { inflight, snapshot }
}

async function assertRouteReady(page: Page, route: string) {
  // Route-specific readiness checks (avoid false PASS when a page is stuck on skeletons/spinners).
  // A page is considered "ready" when it shows either real content OR a clear error/upgrade state.
  const timeoutMs = 25_000
  const started = Date.now()

  const isVisible = async (loc: ReturnType<Page['locator']>) => {
    return loc.first().isVisible().catch(() => false)
  }

  const hasAny = async (...locs: Array<ReturnType<Page['locator']>>) => {
    for (const loc of locs) {
      if (await isVisible(loc)) return true
    }
    return false
  }

  while (Date.now() - started < timeoutMs) {
    // Common: if a visible alert exists, treat that as "ready" (it’s not infinite loading).
    if (await isVisible(page.getByRole('alert'))) return

    if (route === '/fraud') {
      const title = page.getByRole('heading', { name: /fraud detection/i })
      const detectionMethods = page.getByRole('heading', { name: /detection methods/i })
      const recentAlerts = page.getByRole('heading', { name: /recent alerts/i })
      const planChecking = page.getByText(/checking your plan/i)
      const sessionLoading = page.getByText(/loading session/i)

      if (await hasAny(detectionMethods, recentAlerts)) return
      if (await isVisible(title) && !(await hasAny(planChecking, sessionLoading))) {
        // Title rendered but content not yet: keep waiting.
      }
    } else if (route === '/transparency/summary') {
      const title = page.getByRole('heading', { name: /transparency\s*—\s*summary/i })
      const noData = page.getByText(/no sampling data available yet/i)
      const winners = page.getByRole('heading', { name: /winners by source/i })
      if (await hasAny(noData, winners)) return
      if (await isVisible(title)) {
        // If title exists and any stat label exists, consider ready.
        if (await isVisible(page.getByText(/total sampled auctions/i))) return
      }
    } else if (route === '/transparency/auctions') {
      const title = page.getByRole('heading', { name: /transparency\s*—\s*auctions/i })
      const table = page.getByRole('table', { name: /auctions table/i })
      const upgrade = page.getByText(/upgrade to access transparency/i)
      if (await hasAny(table, upgrade)) return
      if (await isVisible(title)) {
        // If title exists and at least one row exists, consider ready.
        const rows = page.locator('table[aria-label="Auctions table"] tbody tr')
        if ((await rows.count().catch(() => 0)) > 0) return
      }
    }

    // Generic readiness: most console routes render a visible heading once hydrated.
    // This prevents false "forever loading" timeouts on pages that don't show alerts.
    const heading = page.getByRole('heading').first()
    const shell = page.locator('aside').first()
    const hasShell = await isVisible(shell)
    const hasHeading = await isVisible(heading)
    if (hasHeading) return
    if (hasShell && (await hasAny(page.getByRole('table'), page.getByRole('button'), page.getByRole('link')))) return

    await page.waitForTimeout(500)
  }

  throw new Error(`Route did not reach a ready state within ${timeoutMs}ms: ${route}`)
}

async function assertNoStuckLoading(page: Page) {
  // Heuristic: avoid ending a step while a visible "Loading..." placeholder is present.
  // (Spinner labels are sr-only and won’t be detected; this targets real UI placeholders.)
  const loadingTexts = ['Loading...', 'Please wait', 'Fetching...', 'Checking your plan', 'Loading session']
  for (const t of loadingTexts) {
    const loc = page.getByText(t, { exact: false }).first()
    if (await loc.isVisible().catch(() => false)) {
      throw new Error(`Possible stuck loading state detected: visible text "${t}"`)
    }
  }
}

function isProbablyExternal(href: string) {
  return href.startsWith('http://') || href.startsWith('https://') || href.startsWith('mailto:') || href.startsWith('tel:')
}

async function clickInternalLinks(page: Page, options?: ClickOptions) {
  const maxClicks = options?.maxClicks ?? 25
  const timeBudgetMs = options?.timeBudgetMs ?? 6_000
  const startedAt = Date.now()

  const links = page.locator('a[href]')
  const count = await links.count().catch(() => 0)

  let clicks = 0
  for (let i = 0; i < count && clicks < maxClicks; i++) {
    if (Date.now() - startedAt > timeBudgetMs) break
    if (page.isClosed()) break

    const link = links.nth(i)
    if (!(await link.isVisible().catch(() => false))) continue

    const href = (await link.getAttribute('href').catch(() => null))
    if (!href) continue
    if (isProbablyExternal(href)) continue
    if (!href.startsWith('/')) continue
    if (href.startsWith('/logout') || href.includes('signout') || href.includes('logout')) continue

    await link.scrollIntoViewIfNeeded().catch(() => {})
    await link.click({ timeout: 750 }).catch(() => {})
    await page.waitForLoadState('domcontentloaded').catch(() => {})
    clicks++
  }
}

async function liveLogin(page: Page, diagnostics?: { badResponses?: string[] }) {
  const email = process.env.LIVE_CONSOLE_EMAIL
  const password = process.env.LIVE_CONSOLE_PASSWORD
  const twofa = process.env.LIVE_CONSOLE_2FA_CODE

  if (!email || !password) {
    throw new Error(
      'Missing LIVE_CONSOLE_EMAIL or LIVE_CONSOLE_PASSWORD.\n' +
      'These tests require REAL production credentials.\n' +
      'See docs/LIVE_TESTING.md for setup instructions.'
    )
  }

  // NOTE: The console's same-origin auth proxy (/api/v1/auth/login) has proven flaky
  // in production builds (historically tied to compression/proxying behavior).
  // Use a direct backend login call and then set session cookies for the console origin.
  await page.goto('/login', { waitUntil: 'networkidle' }).catch(() => page.goto('/login', { waitUntil: 'domcontentloaded' }))

  const origin = new URL(page.url()).origin

  const authResult = await page.evaluate(
    async ({ email, password, twofa }) => {
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
                  resolve({ success: false, error: '2FA is required for this dummy user. Set LIVE_CONSOLE_2FA_CODE and re-run.' })
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

  if (!authResult || !(authResult as any).success || !(authResult as any).token) {
    const extra = diagnostics?.badResponses?.length
      ? `\n\nRecent 4xx responses:\n${diagnostics.badResponses.slice(-8).join('\n\n')}`
      : ''
    throw new Error(`Login failed: ${(authResult as any)?.error || 'unknown'}${extra}`)
  }

  const domain = new URL(origin).hostname
  const isSecure = origin.startsWith('https')
  await page.context().addCookies([
    {
      name: 'access_token',
      value: (authResult as any).token,
      domain,
      path: '/',
      httpOnly: true,
      secure: isSecure,
      sameSite: 'Lax',
    },
    {
      name: 'next-auth.session-token',
      value: (authResult as any).token,
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
        value: (authResult as any).token,
        domain,
        path: '/',
        httpOnly: false,
        secure: true,
        sameSite: 'Lax',
      },
    ])
  }

  // Navigate to dashboard and confirm auth succeeded.
  await page.goto('/dashboard', { waitUntil: 'domcontentloaded', timeout: 30_000 })
  await page.waitForURL('**/dashboard**', { timeout: 15_000 })
  await expect(page).not.toHaveURL(/\/login(\?|$)/)
}

test.describe('Console LIVE — click everything (dummy user)', () => {
  test.describe.configure({ timeout: 240_000 })

  test('login then click through all customer surfaces', async ({ page, baseURL }) => {
    test.skip(!process.env.LIVE_E2E, 'Set LIVE_E2E=1 to enable live click-through')

    if (!baseURL) throw new Error('Playwright baseURL is required for live runs')
    const origin = new URL(baseURL).origin

    // Ensure responsive navigation renders in "desktop" mode.
    await page.setViewportSize({ width: 1440, height: 900 })

    const pageErrors: string[] = []
    const consoleErrors: string[] = []
    const requestFailures: string[] = []
    const sessionEndpointIssues: string[] = []
    const badResponses: string[] = []

    const inflightCapture = startInflightRequestCapture(page, origin)

    fs.mkdirSync('test-results', { recursive: true })
    const debugPath = 'test-results/live-clickthrough-debug.json'
    const steps: Array<{
      at: string
      route: string
      phase: string
      url: string
      hasNav: boolean
      hasAside: boolean
      hasLoginEmail: boolean
      alertText?: string
      sessionProbe?: { ok: boolean; status: number; body: string }
    }> = []

    const flushDebug = () => {
      try {
        fs.writeFileSync(
          debugPath,
          JSON.stringify(
            {
              at: new Date().toISOString(),
              steps,
              pageErrors,
              consoleErrors,
              requestFailures,
              sessionEndpointIssues,
              badResponses,
            },
            null,
            2
          ),
          'utf-8'
        )
      } catch {
        // ignore
      }
    }

    const captureStep = async (route: string, phase: string) => {
      const url = page.url()
      const hasNav = await page.locator('nav').first().isVisible().catch(() => false)
      const hasAside = await page.locator('aside').first().isVisible().catch(() => false)
      const hasLoginEmail = await page.getByLabel('Email').isVisible().catch(() => false)
      const alertText = (await page.getByRole('alert').first().innerText().catch(() => '')).trim()
      const sessionProbe = await page
        .evaluate(async () => {
          try {
            const resp = await fetch('/api/auth/session', { credentials: 'include' })
            const text = await resp.text()
            return { ok: resp.ok, status: resp.status, body: text.slice(0, 800) }
          } catch (e) {
            return { ok: false, status: -1, body: String(e) }
          }
        })
        .catch((e) => ({ ok: false, status: -1, body: String(e) }))

      steps.push({
        at: new Date().toISOString(),
        route,
        phase,
        url,
        hasNav,
        hasAside,
        hasLoginEmail,
        alertText: alertText || undefined,
        sessionProbe,
      })
      while (steps.length > 150) steps.shift()
      flushDebug()
    }

    page.on('pageerror', (err) => pageErrors.push(String(err)))
    page.on('console', (msg) => {
      if (msg.type() !== 'error') return
      const text = msg.text()
      if (text.includes('Failed to load resource')) return
      if (text.includes('favicon')) return
      if (text.includes('Failed to fetch RSC payload')) return
      if (text.includes('[next-auth][error][CLIENT_FETCH_ERROR]') && text.includes('/api/auth/session')) return
      if (text.includes("Failed to execute 'removeChild' on 'Node'") && text.includes('NotFoundError')) return
      consoleErrors.push(text)
    })

    page.on('requestfailed', (req) => {
      const url = req.url()
      const failure = req.failure()?.errorText || 'unknown'
      // Navigations (especially RSC + route-changes) frequently cancel in-flight requests.
      // Treat aborted requests as non-fatal signal to avoid flakiness.
      if (failure.includes('ERR_ABORTED')) return
      if (url.includes('/api/auth/session')) {
        sessionEndpointIssues.push(`requestfailed ${url} (${failure})`)
        return
      }
      requestFailures.push(`requestfailed ${req.method()} ${url} (${failure})`)
    })

    page.on('response', async (res) => {
      const url = res.url()
      // Capture recent 4xxs to help pinpoint where login fails (often CSRF/NextAuth callbacks).
      const status = res.status()
      if (status >= 400 && status < 500 && url.startsWith(origin)) {
        // Some surfaces are intentionally not enabled in production yet.
        if (url.includes('/api/v1/billing/settings')) return

        let snippet = ''
        try {
          snippet = (await res.text()).slice(0, 1200)
          // Redact any JWT-like tokens.
          snippet = snippet.replace(/[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '[JWT_REDACTED]')
        } catch {
          // ignore non-text
        }

        const entry = `${status} ${res.request().method()} ${url}${snippet ? `\n${snippet}` : ''}`
        badResponses.push(entry)
        while (badResponses.length > 12) badResponses.shift()
      }

      if (!url.includes('/api/auth/session')) return
      if (status < 400) return
      let snippet = ''
      try {
        snippet = (await res.text()).slice(0, 1000)
      } catch {
        // ignore non-text
      }
      sessionEndpointIssues.push(`response ${status} ${url}${snippet ? `\n${snippet}` : ''}`)
    })

    const serverErrors = await startServerErrorCapture(page, origin)

    try {
      await liveLogin(page, { badResponses })
      await captureStep('/dashboard', 'post-login')

    // Fail fast if login triggered a backend 5xx.
    expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)

    const routes = [
      '/dashboard',
      '/placements',
      '/adapters',
      '/analytics',
      '/fraud',
      '/supply-chain',
      '/tools/supply-chain-status',
      '/tools/app-ads-inspector',
      '/tools/mediation-debugger',
      '/recon/overview',
      '/recon/deltas',
      '/transparency/summary',
      '/transparency/auctions',
      '/settings',
      '/settings/keys',
      '/settings/sdk',
      '/settings/security',
      '/settings/compliance',
      '/settings/team',
      '/settings/notifications',
      '/settings/networks',
      '/settings/credentials',
      '/settings/billing/settings',
      '/settings/billing/usage',
      '/settings/billing/invoices',
      '/migration-studio',
    ]

      for (const route of routes) {
        await gotoWithRetry(page, route)
        await captureStep(route, 'after-goto')

        const url = page.url()
        if (/\/login|\/signin|\/auth\//.test(url)) {
          await captureStep(route, 'redirected-to-login')
          const inflight = inflightCapture.snapshot().slice(0, 12)
          throw new Error(
            [
              `Redirected to login during route: ${route}`,
              `Current URL: ${url}`,
              sessionEndpointIssues.length
                ? `Session endpoint issues:\n${sessionEndpointIssues.slice(0, 20).join('\n\n')}`
                : null,
              badResponses.length ? `Recent 4xx responses:\n${badResponses.slice(-8).join('\n\n')}` : null,
              requestFailures.length ? `Request failures:\n${requestFailures.slice(0, 20).join('\n')}` : null,
              inflight.length
                ? `Oldest in-flight requests:\n${inflight
                    .map((r) => `${Math.round(r.ageMs / 1000)}s ${r.method} ${r.url}`)
                    .join('\n')}`
                : null,
              consoleErrors.length ? `Console errors:\n${consoleErrors.slice(0, 10).join('\n')}` : null,
              pageErrors.length ? `Page errors:\n${pageErrors.slice(0, 10).join('\n')}` : null,
            ]
              .filter(Boolean)
              .join('\n\n')
          )
        }

      // Catch pages that keep loading forever (Fraud + Transparency are known offenders).
      try {
        await assertRouteReady(page, route)
      } catch (e) {
        const inflight = inflightCapture.snapshot().slice(0, 12)
        throw new Error(
          [
            String(e),
            inflight.length
              ? `\nOldest in-flight requests:\n${inflight
                  .map((r) => `${Math.round(r.ageMs / 1000)}s ${r.method} ${r.url}`)
                  .join('\n')}`
              : null,
          ]
            .filter(Boolean)
            .join('\n')
        )
      }

      // Basic "page rendered" signal: ensure the app shell is present.
      const shellAside = page.locator('aside').first()
      const navByRole = page.getByRole('navigation').first()
      const navByTag = page.locator('nav').first()

      // Give the app shell a moment to hydrate/render.
      await Promise.race([
        shellAside.waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {}),
        navByRole.waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {}),
        navByTag.waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {}),
        page.waitForTimeout(800),
      ])

      const shellVisible =
        (await shellAside.isVisible().catch(() => false)) ||
        (await navByRole.isVisible().catch(() => false)) ||
        (await navByTag.isVisible().catch(() => false))

      if (!shellVisible) {
        const url = page.url()
        const hasLoginEmail = await page.getByLabel('Email').isVisible().catch(() => false)
        const alertText = (await page.getByRole('alert').first().innerText().catch(() => '')).trim()
        const sessionProbe = await page
          .evaluate(async () => {
            try {
              const resp = await fetch('/api/auth/session', { credentials: 'include' })
              const text = await resp.text()
              return { ok: resp.ok, status: resp.status, body: text.slice(0, 800) }
            } catch (e) {
              return { ok: false, status: -1, body: String(e) }
            }
          })
          .catch((e) => ({ ok: false, status: -1, body: String(e) }))

        throw new Error(
          [
            `App shell missing after navigation to ${route}`,
            `Current URL: ${url}`,
            `Login form visible: ${hasLoginEmail}`,
            alertText ? `Alert text: ${alertText}` : null,
            `Session probe: ${JSON.stringify(sessionProbe)}`,
            requestFailures.length ? `Request failures:\n${requestFailures.slice(0, 20).join('\n')}` : null,
            sessionEndpointIssues.length ? `Session endpoint issues:\n${sessionEndpointIssues.slice(0, 20).join('\n\n')}` : null,
            serverErrors.length ? `Server 5xx:\n${serverErrors.slice(0, 10).join('\n\n')}` : null,
            badResponses.length ? `Recent 4xx responses:\n${badResponses.slice(-8).join('\n\n')}` : null,
            inflightCapture.snapshot().length
              ? `In-flight requests:\n${inflightCapture
                  .snapshot()
                  .slice(0, 12)
                  .map((r) => `${Math.round(r.ageMs / 1000)}s ${r.method} ${r.url}`)
                  .join('\n')}`
              : null,
            consoleErrors.length ? `Console errors:\n${consoleErrors.slice(0, 10).join('\n')}` : null,
            pageErrors.length ? `Page errors:\n${pageErrors.slice(0, 10).join('\n')}` : null,
          ]
            .filter(Boolean)
            .join('\n\n')
        )
      }

      await clickVisibleButtons(page, { rounds: 3, maxClicks: 60, timeBudgetMs: 12_000 })
      await clickInternalLinks(page, { maxClicks: 15, timeBudgetMs: 4_000 })
      await closeAnyModal(page)
      await assertNoStuckLoading(page)

        // Stop early with a useful error if any route triggers 5xx.
        expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)
      }

      expect(serverErrors, serverErrors.join('\n\n')).toHaveLength(0)
      expect(pageErrors, pageErrors.join('\n')).toHaveLength(0)
      expect(sessionEndpointIssues, sessionEndpointIssues.join('\n\n')).toHaveLength(0)
      expect(requestFailures, requestFailures.join('\n\n')).toHaveLength(0)
      expect(consoleErrors, consoleErrors.join('\n')).toHaveLength(0)
    } finally {
      flushDebug()
    }
  })
})
