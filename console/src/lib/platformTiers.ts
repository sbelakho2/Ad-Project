export type PlatformTierId = 'starter' | 'growth' | 'scale' | 'enterprise'

export const PLATFORM_TIER_ORDER: PlatformTierId[] = ['starter', 'growth', 'scale', 'enterprise']

const PLATFORM_TIER_ALIASES: Record<string, PlatformTierId> = {
  starter: 'starter',
  indie: 'starter',
  growth: 'growth',
  studio: 'growth',
  scale: 'scale',
  enterprise: 'enterprise',
}

export interface PlatformTierUsageLimits {
  included_impressions: number
  included_api_calls: number
  included_data_transfer_gb: number
  overage_price_impressions_cents: number
  overage_price_api_calls_cents: number
  overage_price_data_transfer_cents: number
}

export const PLATFORM_TIER_LABELS: Record<PlatformTierId, string> = {
  starter: 'Starter',
  growth: 'Growth',
  scale: 'Scale',
  enterprise: 'Enterprise',
}

const PLATFORM_TIER_SUMMARIES: Record<PlatformTierId, string> = {
  starter: '0% platform fee up to $10k/app/month. Basic dashboards only; transparency/VRA/fraud unlock on upgrade.',
  growth: '2.5% platform fee. Unlocks transparency auctions, VRA reconciliation, fraud dashboards, and A/B tools.',
  scale: '2.0% platform fee; 1.25% with annual prepay. Adds RBAC, BI exports, dedicated integration help.',
  enterprise: 'Custom (1.0–1.5%) with SLAs, white-label options, and bespoke data feeds.',
}

export const PLATFORM_TIER_USAGE_LIMITS: Record<PlatformTierId, PlatformTierUsageLimits> = {
  starter: {
    included_impressions: 1_000_000,
    included_api_calls: 100_000,
    included_data_transfer_gb: 50,
    overage_price_impressions_cents: 10,
    overage_price_api_calls_cents: 5,
    overage_price_data_transfer_cents: 10,
  },
  growth: {
    included_impressions: 10_000_000,
    included_api_calls: 1_000_000,
    included_data_transfer_gb: 500,
    overage_price_impressions_cents: 8,
    overage_price_api_calls_cents: 4,
    overage_price_data_transfer_cents: 8,
  },
  scale: {
    included_impressions: 50_000_000,
    included_api_calls: 5_000_000,
    included_data_transfer_gb: 2_500,
    overage_price_impressions_cents: 6,
    overage_price_api_calls_cents: 3,
    overage_price_data_transfer_cents: 6,
  },
  enterprise: {
    included_impressions: 100_000_000,
    included_api_calls: 10_000_000,
    included_data_transfer_gb: 5_000,
    overage_price_impressions_cents: 5,
    overage_price_api_calls_cents: 2,
    overage_price_data_transfer_cents: 5,
  },
}

export const resolvePlatformTierId = (value?: string | null): PlatformTierId | null => {
  if (!value) return null
  return PLATFORM_TIER_ALIASES[String(value).toLowerCase()] ?? null
}

export const formatPlatformTierLabel = (value?: string | null): string => {
  const tier = resolvePlatformTierId(value)
  if (!tier) return 'Unknown tier'
  return PLATFORM_TIER_LABELS[tier]
}

export interface PlatformTierDisplay {
  id: PlatformTierId
  label: string
  summary: string
  usageLimits: PlatformTierUsageLimits
}

export const getPlatformTierDisplay = (value?: string | null): PlatformTierDisplay => {
  const id = resolvePlatformTierId(value) ?? 'starter'
  return {
    id,
    label: PLATFORM_TIER_LABELS[id],
    summary: PLATFORM_TIER_SUMMARIES[id],
    usageLimits: PLATFORM_TIER_USAGE_LIMITS[id],
  }
}
