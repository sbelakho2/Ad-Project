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

function withTimeoutAndAbort<T>(
  promise: Promise<T>,
  options: { timeoutMs: number; signal?: AbortSignal; label: string }
): Promise<T> {
  const { timeoutMs, signal, label } = options
  return new Promise<T>((resolve, reject) => {
    let timeoutHandle: ReturnType<typeof setTimeout> | null = null
    let settled = false

    const finish = (fn: () => void) => {
      if (settled) return
      settled = true
      if (timeoutHandle) clearTimeout(timeoutHandle)
      if (signal) signal.removeEventListener('abort', onAbort)
      fn()
    }

    const onAbort = () => {
      finish(() => reject(new Error(`${label} aborted`)))
    }

    if (signal) {
      if (signal.aborted) {
        onAbort()
        return
      }
      signal.addEventListener('abort', onAbort)
    }

    timeoutHandle = setTimeout(() => {
      finish(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)))
    }, timeoutMs)

    promise.then(
      (value) => finish(() => resolve(value)),
      (error) => finish(() => reject(error))
    )
  })
}

export function usePlanAccess(allowedTiers: PlatformTierId[] = DEFAULT_ALLOWED): PlanAccessResult {
  const { data, isLoading, error } = useQuery({
    queryKey: ['plan-access'],
    queryFn: async ({ signal }) => {
      const usage = await withTimeoutAndAbort(getCurrentUsage({ signal }), {
        signal,
        timeoutMs: 8000,
        label: 'Plan check',
      })
      return usage.subscription.plan_type
    },
    staleTime: 5 * 60 * 1000,
    retry: 1,
    retryDelay: 500,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })

  const plan = data ?? null
  const resolvedError = error ? error.message : null
  const allowed = useMemo(() => {
    // Plan gating is UX only; backend still enforces access.
    // Fail-open if the plan check is temporarily unavailable to avoid blocking core pages.
    if (!plan) return Boolean(resolvedError)
    return allowedTiers.includes(plan)
  }, [allowedTiers, plan, resolvedError])

  return {
    plan,
    allowed,
    isLoading,
    error: resolvedError,
  }
}
