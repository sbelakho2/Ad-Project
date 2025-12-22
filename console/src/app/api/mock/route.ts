// Mock API for local testing
// This provides realistic data for all dashboard features
export const dynamic = 'force-dynamic'

import { NextResponse } from 'next/server'

const allowMockApi = process.env.NODE_ENV !== 'production' && process.env.NEXT_PUBLIC_USE_MOCK_API === 'true'

// Helper to generate random data
const random = (min: number, max: number) => Math.floor(Math.random() * (max - min + 1)) + min
const randomFloat = (min: number, max: number, decimals = 2) =>
  parseFloat((Math.random() * (max - min) + min).toFixed(decimals))

const now = new Date()
const thirtyDaysAgo = new Date(now.getTime() - 30 * 24 * 60 * 60 * 1000)

// Generate time series data
function generateTimeSeries(days: number) {
  const data = []
  const now = new Date()
  for (let i = days - 1; i >= 0; i--) {
    const date = new Date(now)
    date.setDate(date.getDate() - i)
    data.push({
      date: date.toISOString().split('T')[0],
      revenue: randomFloat(1000, 5000),
      impressions: random(50000, 200000),
      clicks: random(500, 2000),
      ecpm: randomFloat(2, 8),
      fillRate: randomFloat(0.75, 0.95),
    })
  }
  return data
}

// Mock data generators
const mockData = {
  revenueSummary: {
    totalRevenue: randomFloat(45000, 85000),
    totalImpressions: random(2000000, 5000000),
    totalClicks: random(25000, 75000),
    averageEcpm: randomFloat(3.5, 7.5),
    averageFillRate: randomFloat(0.82, 0.94),
    periodStart: thirtyDaysAgo.toISOString(),
    periodEnd: now.toISOString(),
    revenueChangePercent: randomFloat(-0.15, 0.25),
    impressionsChangePercent: randomFloat(-0.1, 0.3),
    ecpmChangePercent: randomFloat(-0.08, 0.12),
    fillRateChangePercent: randomFloat(-0.05, 0.1),
  },

  revenueTimeSeries: generateTimeSeries(30),

  placements: [
    {
      id: 'plc_1',
      name: 'Main Menu Banner',
      type: 'banner',
      status: 'active',
      format: '320x50',
      platformId: 'ios',
      publisherId: 'pub_1',
      createdAt: '2024-01-15T10:00:00Z',
      updatedAt: now.toISOString(),
    },
    {
      id: 'plc_2',
      name: 'Gameplay Interstitial',
      type: 'interstitial',
      status: 'active',
      format: 'full-screen',
      platformId: 'android',
      publisherId: 'pub_1',
      createdAt: '2024-01-15T10:00:00Z',
      updatedAt: now.toISOString(),
    },
    {
      id: 'plc_3',
      name: 'Reward Video - Extra Lives',
      type: 'rewarded',
      status: 'active',
      format: 'rewarded',
      platformId: 'ios',
      publisherId: 'pub_1',
      createdAt: '2024-02-01T10:00:00Z',
      updatedAt: now.toISOString(),
    },
  ],

  adapters: [
    {
      id: 'adp_1',
      name: 'Google AdMob',
      network: 'admob',
      status: 'active',
      priority: 1,
      ecpm: randomFloat(5, 10),
      fillRate: randomFloat(0.88, 0.96),
      requestCount: random(900000, 1500000),
      impressionCount: random(800000, 1400000),
      placementId: 'plc_1',
      createdAt: '2024-01-15T10:00:00Z',
      updatedAt: now.toISOString(),
    },
    {
      id: 'adp_2',
      name: 'AppLovin MAX',
      network: 'applovin',
      status: 'active',
      priority: 2,
      ecpm: randomFloat(4.5, 9),
      fillRate: randomFloat(0.85, 0.93),
      requestCount: random(700000, 1200000),
      impressionCount: random(600000, 1100000),
      placementId: 'plc_2',
      createdAt: '2024-02-10T10:00:00Z',
      updatedAt: now.toISOString(),
    },
    {
      id: 'adp_3',
      name: 'Meta Audience Network',
      network: 'facebook',
      status: 'active',
      priority: 3,
      ecpm: randomFloat(4, 8),
      fillRate: randomFloat(0.78, 0.88),
      requestCount: random(550000, 1000000),
      impressionCount: random(450000, 850000),
      placementId: 'plc_2',
      createdAt: '2024-03-01T10:00:00Z',
      updatedAt: now.toISOString(),
    },
    {
      id: 'adp_4',
      name: 'IronSource',
      network: 'ironsource',
      status: 'active',
      priority: 4,
      ecpm: randomFloat(3.5, 7),
      fillRate: randomFloat(0.75, 0.85),
      requestCount: random(400000, 800000),
      impressionCount: random(350000, 650000),
      placementId: 'plc_3',
      createdAt: '2024-03-15T10:00:00Z',
      updatedAt: now.toISOString(),
    },
  ],

  fraudStats: {
    publisherId: 'pub_1',
    timeWindow: '24h',
    totalRequests: random(2000000, 5000000),
    fraudRequests: random(10000, 50000),
    fraudRate: randomFloat(0.005, 0.025),
    givtDetections: random(3000, 15000),
    sivtDetections: random(2000, 10000),
    mlDetections: random(3000, 15000),
    anomalyDetections: random(2000, 10000),
    blockedRequests: random(5000, 20000),
    flaggedRequests: random(1000, 5000),
    averageFraudScore: randomFloat(35, 75),
    topFraudTypes: ['Bot Traffic', 'Click Injection', 'SDK Spoofing'],
    lastUpdated: now.toISOString(),
  },

  fraudAlerts: [
    {
      id: 'alert_1',
      timestamp: new Date(now.getTime() - 2 * 60 * 60 * 1000).toISOString(),
      alertType: 'givt',
      severity: 'high',
      publisherId: 'pub_1',
      deviceId: 'device_botfarm',
      ipAddress: '192.168.1.12',
      fraudScore: randomFloat(70, 95),
      detectionType: 'ml_based',
      action: 'block',
      metadata: {
        message: 'Unusual bot traffic detected from IP range 192.168.1.0/24',
        affectedImpressions: random(1000, 5000),
      },
    },
    {
      id: 'alert_2',
      timestamp: new Date(now.getTime() - 5 * 60 * 60 * 1000).toISOString(),
      alertType: 'ml_fraud',
      severity: 'critical',
      publisherId: 'pub_1',
      deviceId: 'placement_plc_2',
      ipAddress: '10.10.0.5',
      fraudScore: randomFloat(80, 98),
      detectionType: 'ml_based',
      action: 'block',
      metadata: {
        message: 'Click injection pattern detected on placement plc_2',
        affectedImpressions: random(500, 2000),
      },
    },
    {
      id: 'alert_3',
      timestamp: new Date(now.getTime() - 12 * 60 * 60 * 1000).toISOString(),
      alertType: 'anomaly',
      severity: 'medium',
      publisherId: 'pub_1',
      deviceId: 'global',
      ipAddress: '198.51.100.55',
      fraudScore: randomFloat(45, 65),
      detectionType: 'anomaly',
      action: 'monitor',
      metadata: {
        message: 'Abnormal eCPM spike detected (+250% in last hour)',
        affectedImpressions: random(200, 1000),
      },
    },
  ],

  payoutHistory: [
    {
      id: 'payout_1',
      amount: randomFloat(8500, 12500),
      currency: 'USD',
      status: 'completed',
      method: 'stripe',
      scheduledFor: new Date(Date.now() - 15 * 24 * 60 * 60 * 1000).toISOString(),
      processedAt: new Date(Date.now() - 14 * 24 * 60 * 60 * 1000).toISOString(),
    },
    {
      id: 'payout_2',
      amount: randomFloat(9000, 14000),
      currency: 'USD',
      status: 'completed',
      method: 'wire',
      scheduledFor: new Date(Date.now() - 22 * 24 * 60 * 60 * 1000).toISOString(),
      processedAt: new Date(Date.now() - 21 * 24 * 60 * 60 * 1000).toISOString(),
    },
    {
      id: 'payout_3',
      amount: randomFloat(7500, 11000),
      currency: 'USD',
      status: 'processing',
      method: 'stripe',
      scheduledFor: new Date(Date.now() + 2 * 24 * 60 * 60 * 1000).toISOString(),
    },
  ],

  upcomingPayout: {
    id: 'payout_upcoming',
    amount: randomFloat(5000, 9000),
    currency: 'USD',
    status: 'pending',
    method: 'stripe',
    scheduledFor: new Date(Date.now() + 5 * 24 * 60 * 60 * 1000).toISOString(),
  },

  analytics: {
    impressionsByPlacement: [
      { placement: 'Main Menu Banner', impressions: random(800000, 1200000), revenue: randomFloat(8000, 15000) },
      { placement: 'Gameplay Interstitial', impressions: random(600000, 1000000), revenue: randomFloat(12000, 20000) },
      { placement: 'Reward Video', impressions: random(400000, 800000), revenue: randomFloat(15000, 30000) },
    ],
    impressionsByAdapter: [
      { adapter: 'Google AdMob', impressions: random(1000000, 1800000), revenue: randomFloat(15000, 30000) },
      { adapter: 'AppLovin MAX', impressions: random(800000, 1500000), revenue: randomFloat(12000, 25000) },
      { adapter: 'Meta Audience', impressions: random(600000, 1200000), revenue: randomFloat(8000, 18000) },
      { adapter: 'IronSource', impressions: random(400000, 900000), revenue: randomFloat(6000, 15000) },
    ],
  },
}

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url)
  const endpoint = searchParams.get('endpoint')

  if (!allowMockApi) {
    return NextResponse.json({ error: 'Mock API disabled' }, { status: 404 })
  }

  // Simulate network delay
  await new Promise(resolve => setTimeout(resolve, random(100, 500)))

  try {
    switch (endpoint) {
      case 'csrf': {
        const token = 'mock-csrf-token'
        const response = NextResponse.json({ token })
        response.cookies.set('XSRF-TOKEN', token, {
          httpOnly: false,
          sameSite: 'lax',
          path: '/',
        })
        return response
      }

      case 'revenue-summary':
        return NextResponse.json(mockData.revenueSummary)
      
      case 'revenue-timeseries':
        return NextResponse.json(mockData.revenueTimeSeries)
      
      case 'placements':
        return NextResponse.json({
          data: mockData.placements,
          total: mockData.placements.length,
          page: 1,
          pageSize: mockData.placements.length,
          hasMore: false,
        })
      
      case 'adapters':
        return NextResponse.json(mockData.adapters)
      
      case 'fraud-stats':
        return NextResponse.json(mockData.fraudStats)
      
      case 'fraud-alerts':
        return NextResponse.json({ alerts: mockData.fraudAlerts })
      
      case 'payout-history':
        return NextResponse.json({
          data: mockData.payoutHistory,
          total: mockData.payoutHistory.length,
          page: 1,
          pageSize: mockData.payoutHistory.length,
          hasMore: false,
        })
      
      case 'payout-upcoming':
        return NextResponse.json(mockData.upcomingPayout)
      
      case 'analytics':
        return NextResponse.json(mockData.analytics)

      case 'migration-experiments':
        return NextResponse.json({
          success: true,
          data: [
            {
              id: 'exp-1',
              name: 'Unity to Apex Migration',
              status: 'active',
              mirror_percent: 5,
              created_at: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(),
              updated_at: new Date().toISOString(),
              guardrails: {
                latency_budget_ms: 200,
                revenue_floor_percent: -5,
                max_error_rate_percent: 1,
                min_impressions: 1000,
              },
            },
          ],
        })

      case 'migration-experiment':
        return NextResponse.json({
          success: true,
          data: {
            id: 'exp-1',
            name: 'Unity to Apex Migration',
            status: 'active',
            mirror_percent: 5,
            created_at: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(),
            updated_at: new Date().toISOString(),
            guardrails: {
              latency_budget_ms: 200,
              revenue_floor_percent: -5,
              max_error_rate_percent: 1,
              min_impressions: 1000,
            },
          },
        })
      
      default:
        return NextResponse.json({ error: 'Unknown endpoint' }, { status: 404 })
    }
  } catch (error) {
    console.error('Mock API error:', error)
    return NextResponse.json({ error: 'Internal server error' }, { status: 500 })
  }
}
