"use client"

import { useMemo, useEffect } from 'react'
import { useRouter } from 'next/navigation'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import axios from 'axios'
import { apiClient, AUTH_UNAUTHORIZED_EVENT, setActiveTenantId } from './api-client'
import { clearInvoicePdfCache } from './billing'
import { readXsrfCookie } from './csrf'

export type SessionUser = {
  userId: string
  publisherId: string
  email: string
  role?: 'admin' | 'publisher' | 'readonly'
  permissions?: string[]
  locale?: string | null
}

type UseSessionOptions = {
  redirectOnUnauthorized?: boolean
  redirectOnLogout?: boolean
  loginPath?: string
}

const DEFAULT_LOGIN_PATH = '/login'

let MOCK_SESSION: SessionUser | null = null

function readMockSession(): SessionUser | null {
  // Preferred: build-time env (available to Next client bundles via NEXT_PUBLIC_*)
  if (process.env.NEXT_PUBLIC_E2E_SESSION) {
    try {
      return JSON.parse(process.env.NEXT_PUBLIC_E2E_SESSION) as SessionUser
    } catch {
      return null
    }
  }

  // Fallback for E2E: runtime-injected session (Playwright can set this before navigation).
  // Never allow this in production builds.
  if (process.env.NODE_ENV !== 'production' && typeof window !== 'undefined') {
    const candidate = (window as any).__APEX_E2E_SESSION__ as SessionUser | undefined
    if (candidate && typeof candidate === 'object' && typeof candidate.userId === 'string') {
      return candidate
    }
  }

  return null
}

MOCK_SESSION = readMockSession()

function resolveSessionScope(): string {
  if (typeof window === 'undefined') return 'ssr'
  // Session cache key must be stable for the lifetime of the tab.
  // Using the XSRF cookie here can cause the key to churn when the token rotates,
  // which makes Navigation appear to "keep loading".
  return `client:${window.location.host}`
}

export function useSession(options?: UseSessionOptions) {
  const {
    redirectOnUnauthorized = true,
    redirectOnLogout = true,
    loginPath = DEFAULT_LOGIN_PATH,
  } = options ?? {}

  const router = useRouter()
  const queryClient = useQueryClient()
  const sessionScope = useMemo(() => resolveSessionScope(), [])
  const sessionQueryKey = useMemo(() => ['session', 'me', sessionScope] as const, [sessionScope])

  const isMockedSession = Boolean(MOCK_SESSION)

  const query = useQuery({
    queryKey: sessionQueryKey,
    queryFn: async (): Promise<SessionUser | null> => {
      try {
        const res = await apiClient.get('/auth/me')
        if (res?.data?.success) return res.data.data as SessionUser
        return null
      } catch (error) {
        if (axios.isAxiosError(error) && error.response?.status === 401) {
          return null
        }
        throw error
      }
    },
    retry: false,
    staleTime: 5 * 60 * 1000,
    enabled: !isMockedSession,
  })

  useEffect(() => {
    if (isMockedSession || typeof window === 'undefined') return

    const handleUnauthorized = () => {
      queryClient.setQueryData(sessionQueryKey, null)
      if (!redirectOnUnauthorized) return

      const nextPath = window.location.pathname + window.location.search + window.location.hash
      if (nextPath.startsWith(loginPath)) return
      router.replace(`${loginPath}?next=${encodeURIComponent(nextPath)}`)
    }

    window.addEventListener(AUTH_UNAUTHORIZED_EVENT, handleUnauthorized)
    return () => window.removeEventListener(AUTH_UNAUTHORIZED_EVENT, handleUnauthorized)
  }, [isMockedSession, loginPath, queryClient, redirectOnUnauthorized, router, sessionQueryKey])

  const logout = useMutation({
    mutationKey: ['session', 'logout', sessionScope],
    mutationFn: async () => {
      if (isMockedSession) return
      await apiClient.post('/auth/logout')
    },
    onSuccess: async () => {
      if (!isMockedSession) {
        queryClient.setQueryData(sessionQueryKey, null)
        clearInvoicePdfCache()
        await queryClient.invalidateQueries({ queryKey: sessionQueryKey })
        if (redirectOnLogout && typeof window !== 'undefined') {
          const nextPath = window.location.pathname + window.location.search + window.location.hash
          router.push(`${loginPath}?next=${encodeURIComponent(nextPath)}`)
        }
      }
    },
  })

  const resolvedUser = (MOCK_SESSION as SessionUser | null) ?? query.data ?? null

  useEffect(() => {
    setActiveTenantId(resolvedUser?.publisherId ?? null)
    return () => {
      setActiveTenantId(null)
    }
  }, [resolvedUser?.publisherId])

  return {
    user: resolvedUser,
    isLoading: isMockedSession ? false : query.isLoading,
    error: query.error,
    refetch: query.refetch,
    logout,
    queryKey: sessionQueryKey,
  }
}
