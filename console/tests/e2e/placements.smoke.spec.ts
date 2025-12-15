import { test, expect, Page, Route } from '@playwright/test'

type PlacementRecord = {
  id: string
  name: string
  type: 'banner' | 'interstitial' | 'rewarded'
  format: string
  platformId: string
  status: 'active' | 'paused' | 'archived'
  publisherId: string
  createdAt: string
  updatedAt: string
}

const CURRENT_PUBLISHER_ID = 'pub-test-123'

const FEATURE_FLAGS = {
  transparency: true,
  billing: true,
  migrationStudio: true,
}
const seedPlacement: PlacementRecord = {
  id: 'plc-home-banner',
  name: 'Home Screen Banner',
  type: 'banner',
  format: '320x50',
  platformId: 'com.demo.game',
  status: 'active',
  publisherId: CURRENT_PUBLISHER_ID,
  createdAt: new Date('2024-01-01T00:00:00Z').toISOString(),
  updatedAt: new Date('2024-01-05T00:00:00Z').toISOString(),
}

const foreignPlacement: PlacementRecord = {
  id: 'plc-foreign-only',
  name: 'Foreign Only Placement',
  type: 'rewarded',
  format: 'fullscreen-video',
  platformId: 'com.other.app',
  status: 'active',
  publisherId: 'pub-foreign-999',
  createdAt: new Date('2024-01-02T00:00:00Z').toISOString(),
  updatedAt: new Date('2024-01-04T00:00:00Z').toISOString(),
}

const respondJson = (route: Route, status: number, body: unknown) => {
  return route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  })
}

test.describe('Placements CRUD smoke', () => {
  test.use({ storageState: undefined })

  test('publisher scoped placement lifecycle', async ({ page }) => {
    page.on('console', (message) => {
      console.log('[browser]', message.type(), message.text())
    })
    const placementStore = new Map<string, PlacementRecord>([
      [seedPlacement.id, { ...seedPlacement }],
      [foreignPlacement.id, { ...foreignPlacement }],
    ])

    let createCalled = 0
    let updateCalled = 0
    let deleteCalled = 0
    let unauthorizedLookupCount = 0

    await stubPlacementApi(page, placementStore, {
      onCreate: () => {
        createCalled += 1
      },
      onUpdate: () => {
        updateCalled += 1
      },
      onDelete: () => {
        deleteCalled += 1
      },
      onUnauthorizedLookup: () => {
        unauthorizedLookupCount += 1
      },
    })

    await page.goto('/placements')
    await expect(page).toHaveURL(/\/placements$/)
    await expect(page.getByRole('heading', { name: 'Ad Placements' })).toBeVisible()
    await expect(page.getByText(seedPlacement.name)).toBeVisible()
    await expect(page.getByText(foreignPlacement.name)).toHaveCount(0)

    await page.getByRole('link', { name: 'Create Placement' }).click()
    await expect(page).toHaveURL(/\/placements\/new/)

    await page.getByLabel('Placement Name').fill('Test Rewarded Placement')
    await page.getByLabel('Platform ID (App ID)').fill('com.example.rewarded')
    await page.getByLabel('Ad Type').selectOption('rewarded')
    await page.getByLabel('Ad Format').selectOption('rewarded-interstitial')
    await page.getByRole('button', { name: 'Create Placement' }).click()

    await expect(page.getByRole('heading', { level: 1, name: 'Test Rewarded Placement' })).toBeVisible()

    await page.getByRole('button', { name: 'Pause' }).click()
    await expect(page.getByRole('button', { name: 'Activate' })).toBeVisible()

    await page.getByRole('button', { name: 'Delete' }).click()
    const confirmDialog = page.locator('[role="dialog"]')
    await expect(confirmDialog).toBeVisible()
    await confirmDialog.getByLabel('Confirmation keyword').fill('DELETE')
    await confirmDialog.getByRole('button', { name: /^Delete$/ }).click()

    await expect(page).toHaveURL(/\/placements$/)
    await expect(page.getByText('Test Rewarded Placement')).toHaveCount(0)
    await expect(page.getByText(seedPlacement.name)).toBeVisible()

    await page.goto(`/placements/${foreignPlacement.id}`)
    await expect(page.getByText('Placement not found')).toBeVisible()

    expect(createCalled).toBe(1)
    expect(updateCalled).toBeGreaterThanOrEqual(1)
    expect(deleteCalled).toBe(1)
    expect(unauthorizedLookupCount).toBe(1)
  })
})

function pathFrom(url: string) {
  try {
    return new URL(url).pathname
  } catch {
    return url
  }
}

async function stubPlacementApi(
  page: Page,
  placementStore: Map<string, PlacementRecord>,
  hooks: {
    onCreate: () => void
    onUpdate: () => void
    onDelete: () => void
    onUnauthorizedLookup: () => void
  }
) {
  await page.route('**/auth/csrf', async (route) => {
    return route.fulfill({
      status: 200,
      headers: { 'set-cookie': 'XSRF-TOKEN=csrf-token; Path=/; HttpOnly' },
      contentType: 'application/json',
      body: JSON.stringify({ token: 'csrf-token' }),
    })
  })

  await page.route('**/meta/features', async (route) => {
    console.log('[mock] GET /meta/features')
    return respondJson(route, 200, { data: FEATURE_FLAGS, success: true })
  })

  // Mock auth/me endpoint required by Navigation component
  await page.route('**/auth/me', async (route) => {
    console.log('[mock] GET /auth/me')
    return respondJson(route, 200, {
      success: true,
      data: {
        userId: 'usr-test',
        email: 'test@example.com',
        role: 'publisher',
        publisherId: CURRENT_PUBLISHER_ID,
      },
    })
  })

  // Mock billing usage endpoint for plan access
  await page.route('**/billing/usage/**', async (route) => {
    return respondJson(route, 200, {
      current_period: {
        start: new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString(),
        end: new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString(),
        impressions: 100000,
        api_calls: 50000,
        data_transfer_gb: 10,
      },
      overages: {
        impressions: { amount: 0, cost: 0 },
        api_calls: { amount: 0, cost: 0 },
        data_transfer: { amount: 0, cost: 0 },
        total_overage_cost: 0,
      },
      subscription: {
        plan_type: 'growth',
        included_impressions: 500000,
        included_api_calls: 100000,
        included_data_transfer_gb: 50,
      },
    })
  })

  // Only intercept API calls, not page navigations
  // Match /api/v1/placements with or without query params
  await page.route(/\/api\/v1\/placements(\?|$)/, async (route) => {
    const request = route.request()
    const method = request.method().toUpperCase()

    if (method === 'GET') {
      const data = Array.from(placementStore.values()).filter(
        (placement) => placement.publisherId === CURRENT_PUBLISHER_ID
      )
      console.log('[mock] GET /api/v1/placements - returning', data.length, 'items')
      // Match the expected response structure - items instead of data
      return respondJson(route, 200, {
        success: true,
        data: {
          items: data,
          total: data.length,
          page: 1,
          pageSize: data.length || 20,
          hasMore: false,
        },
      })
    }

    if (method === 'POST') {
      hooks.onCreate()
      const payload = JSON.parse(request.postData() || '{}') as Partial<PlacementRecord>
      const timestamp = new Date().toISOString()
      const newPlacement: PlacementRecord = {
        id: `plc-generated-${Date.now()}`,
        name: payload.name || 'Untitled Placement',
        type: (payload.type as PlacementRecord['type']) || 'banner',
        format: payload.format || '320x50',
        platformId: payload.platformId || 'com.example.app',
        status: 'active',
        publisherId: CURRENT_PUBLISHER_ID,
        createdAt: timestamp,
        updatedAt: timestamp,
      }
      placementStore.set(newPlacement.id, newPlacement)
      return respondJson(route, 200, newPlacement)
    }

    return route.continue()
  })

  await page.route('**/api/v1/placements/formats*', async (route) => {
    console.log('[mock] GET /api/v1/placements/formats')
    return respondJson(route, 200, {
      banner: ['320x50', '300x250', '728x90'],
      interstitial: ['fullscreen'],
      rewarded: ['rewarded-video', 'rewarded-interstitial'],
    })
  })

  await page.route(/\/api\/v1\/placements\/[^/]+$/, async (route) => {
    const request = route.request()
    const method = request.method().toUpperCase()
    const pathname = pathFrom(request.url())
    const placementMatch = pathname.match(/\/placements\/([^/]+)$/)

    if (!placementMatch) {
      return route.continue()
    }

    const placementId = placementMatch[1]
    const existing = placementStore.get(placementId)

    if (method === 'GET') {
      if (!existing || existing.publisherId !== CURRENT_PUBLISHER_ID) {
        if (existing && existing.publisherId !== CURRENT_PUBLISHER_ID) {
          hooks.onUnauthorizedLookup()
        }
        return respondJson(route, 200, null)
      }
      console.log('[mock] GET /placements/', placementId)
      return respondJson(route, 200, existing)
    }

    if (method === 'PUT') {
      if (!existing || existing.publisherId !== CURRENT_PUBLISHER_ID) {
        return respondJson(route, 404, { message: 'Not found' })
      }
      hooks.onUpdate()
      const payload = JSON.parse(request.postData() || '{}') as Partial<PlacementRecord>
      const updated: PlacementRecord = {
        ...existing,
        ...payload,
        updatedAt: new Date().toISOString(),
      }
      placementStore.set(placementId, updated)
      console.log('[mock] PUT /placements/', placementId)
      return respondJson(route, 200, updated)
    }

    if (method === 'DELETE') {
      if (!existing || existing.publisherId !== CURRENT_PUBLISHER_ID) {
        return respondJson(route, 404, { message: 'Not found' })
      }
      hooks.onDelete()
      placementStore.delete(placementId)
      console.log('[mock] DELETE /placements/', placementId)
      return route.fulfill({ status: 204, body: '' })
    }

    return route.continue()
  })
}
