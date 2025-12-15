"use client"

import { useCallback, useEffect, useRef, useState } from 'react'
import { useRouter, usePathname } from 'next/navigation'
import { isFeatureEnabled } from '@/lib/featureFlags'

/**
 * OPERATOR AUTHENTICATION GATE
 * 
 * This hook gates the Admin Control Panel (operator cockpit) for SYSTEM OPERATORS ONLY.
 * It is completely separate from customer authentication and the customer RBAC system.
 * 
 * ╔════════════════════════════════════════════════════════════════════════════╗
 * ║ CRITICAL SEPARATION: OPERATOR vs CUSTOMER AUTHENTICATION                   ║
 * ╠════════════════════════════════════════════════════════════════════════════╣
 * ║                                                                            ║
 * ║ OPERATOR (System Admin)          │ CUSTOMER (Console User)                 ║
 * ║ ─────────────────────────────────┼───────────────────────────────────────  ║
 * ║ Uses: OPERATOR_EMAILS whitelist  │ Uses: NextAuth session + JWT            ║
 * ║ Uses: OPERATOR_SECRET            │ Uses: Customer RBAC (admin/pub/readonly)║
 * ║ Access: /admin/* routes          │ Access: All other console routes        ║
 * ║ Controls: System infrastructure  │ Controls: Organization resources        ║
 * ║ Scope: Company-wide privileged   │ Scope: Per-organization features        ║
 * ║                                                                            ║
 * ║ Customer 'admin' role ≠ Operator │ No customer role grants operator access ║
 * ╚════════════════════════════════════════════════════════════════════════════╝
 * 
 * The admin panel exposes company-privileged data and system controls that must be
 * restricted to authorized operators only.
 * 
 * Authentication methods (in order of precedence):
 * 1. OPERATOR_EMAILS environment variable - comma-separated list of operator email addresses
 * 2. OPERATOR_SECRET - a secret key that operators must provide (stored in session/localStorage)
 * 
 * For remote-ready future: Can extend to support OAuth/SSO with operator identity provider.
 */

type UseOperatorGateOptions = {
  unauthorizedRedirect?: string
  disableRedirects?: boolean
}

const DEFAULT_OPTIONS: UseOperatorGateOptions = {
  unauthorizedRedirect: '/403',
  disableRedirects: false,
}

type OperatorSession = {
  identifier: string
}

async function fetchOperatorSession(): Promise<OperatorSession | null> {
  try {
    const response = await fetch('/api/operator/session', { credentials: 'include' })
    if (!response.ok) return null
    const data = (await response.json()) as { identifier?: string }
    if (!data?.identifier) return null
    return { identifier: data.identifier }
  } catch {
    return null
  }
}

export function useAdminGate(options?: UseOperatorGateOptions) {
  const { unauthorizedRedirect, disableRedirects } = {
    ...DEFAULT_OPTIONS,
    ...options,
  } as Required<UseOperatorGateOptions>

  const router = useRouter()
  const pathname = usePathname()
  const [isClient, setIsClient] = useState(false)
  const [isCheckingAccess, setIsCheckingAccess] = useState(true)
  const [isOperator, setIsOperator] = useState(false)
  const [operatorIdentifier, setOperatorIdentifier] = useState<string | null>(null)
  const lastRedirectRef = useRef<string | null>(null)
  const guardEnabled = isFeatureEnabled('requireAdminGuard')

  useEffect(() => {
    setIsClient(true)
  }, [])

  // Check operator access on mount
  useEffect(() => {
    if (!isClient) return

    const checkOperatorAccess = async () => {
      setIsCheckingAccess(true)

      const session = await fetchOperatorSession()
      if (session) {
        setIsOperator(true)
        setOperatorIdentifier(session.identifier)
        setIsCheckingAccess(false)
        return
      }

      // No valid operator access
      setIsOperator(false)
      setIsCheckingAccess(false)
    }

    checkOperatorAccess()
  }, [isClient])

  const redirect = useCallback((target: string, reason: string) => {
    if (!guardEnabled || disableRedirects || typeof window === 'undefined') return
    if (lastRedirectRef.current === reason) return
    if (pathname === target) return
    lastRedirectRef.current = reason
    router.replace(target)
  }, [disableRedirects, guardEnabled, pathname, router])

  // Redirect unauthorized users
  useEffect(() => {
    if (!isClient || isCheckingAccess || !guardEnabled) return

    if (!isOperator) {
      redirect(unauthorizedRedirect!, 'forbidden')
      return
    }

    lastRedirectRef.current = null
  }, [guardEnabled, isClient, isCheckingAccess, isOperator, redirect, unauthorizedRedirect])

  return {
    // For backward compatibility, we expose 'user' as a minimal object
    // but this is NOT a customer user - it's operator identity
    user: isOperator ? { email: operatorIdentifier, role: 'operator' as const } : null,
    isAdmin: isOperator, // renamed semantically but kept for compat
    isOperator,
    isLoading: isCheckingAccess || !isClient,
  }
}

/**
 * Utility to set operator credentials for session
 * Used by the operator login page/mechanism
 */
export async function operatorLogin(secret: string, identifier?: string): Promise<boolean> {
  try {
    const res = await fetch('/api/operator/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ secret, identifier }),
    })
    return res.ok
  } catch {
    return false
  }
}

export async function operatorLogout(): Promise<void> {
  try {
    await fetch('/api/operator/logout', {
      method: 'POST',
      credentials: 'include',
    })
  } catch {
    // ignore
  }
}
