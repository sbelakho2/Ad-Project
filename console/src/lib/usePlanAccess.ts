import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { getCurrentUsage } from './billing'
import type { PlatformTierId } from './platformTiers'

const DEFAULT_ALLOWED: PlatformTierId[] = ['growth', 'scale', 'enterprise']

type PlanAccessResult = {
  plan: PlatformTierId | null
  allowed: boolean
  isLoading: boolean
  error?: string | null
}

export function usePlanAccess(allowedTiers: PlatformTierId[] = DEFAULT_ALLOWED): PlanAccessResult {
  const { data, isLoading, error } = useQuery({
    queryKey: ['plan-access'],
    queryFn: async () => {
      const usage = await getCurrentUsage()
      return usage.subscription.plan_type
    },
    staleTime: 5 * 60 * 1000,
    retry: 1,
  })

  const plan = data ?? null
  const allowed = useMemo(() => {
    if (!plan) return false
    return allowedTiers.includes(plan)
  }, [allowedTiers, plan])

  return {
    plan,
    allowed,
    isLoading,
    error: error ? error.message : null,
  }
}
