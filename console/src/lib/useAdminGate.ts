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
 * CRITICAL: Customer roles (viewer, admin, etc.) have NO access to the operator cockpit.
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

// Operator emails are defined server-side via environment variable
// This list is fetched at build time or via API for security
const OPERATOR_EMAILS = (process.env.NEXT_PUBLIC_OPERATOR_EMAILS || '')
  .split(',')
  .map(email => email.trim().toLowerCase())
  .filter(Boolean)

// Check if user's email is in the operator whitelist
function isOperatorEmail(email: string | undefined | null): boolean {
  if (!email) return false
  // If no operator emails are configured, deny all access (fail-secure)
  if (OPERATOR_EMAILS.length === 0) return false
  return OPERATOR_EMAILS.includes(email.toLowerCase())
}

// Check if operator secret is provided and valid
function hasValidOperatorSecret(): boolean {
  if (typeof window === 'undefined') return false
  const secret = sessionStorage.getItem('operator_secret') || localStorage.getItem('operator_secret')
  const expectedSecret = process.env.NEXT_PUBLIC_OPERATOR_SECRET
  if (!expectedSecret) return false
  return secret === expectedSecret
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
  const [operatorEmail, setOperatorEmail] = useState<string | null>(null)
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

      // Method 1: Check for valid operator secret in session
      if (hasValidOperatorSecret()) {
        const email = sessionStorage.getItem('operator_email') || 'operator@system'
        setIsOperator(true)
        setOperatorEmail(email)
        setIsCheckingAccess(false)
        return
      }

      // Method 2: Check if authenticated user's email is in operator whitelist
      // This requires fetching the current session from our auth endpoint
      try {
        const response = await fetch('/api/auth/session', { credentials: 'include' })
        if (response.ok) {
          const session = await response.json()
          if (session?.user?.email && isOperatorEmail(session.user.email)) {
            setIsOperator(true)
            setOperatorEmail(session.user.email)
            setIsCheckingAccess(false)
            return
          }
        }
      } catch (e) {
        // Silent fail - will redirect to unauthorized
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
    user: isOperator ? { email: operatorEmail, role: 'operator' as const } : null,
    isAdmin: isOperator, // renamed semantically but kept for compat
    isOperator,
    isLoading: isCheckingAccess || !isClient,
  }
}

/**
 * Utility to set operator credentials for session
 * Used by the operator login page/mechanism
 */
export function setOperatorCredentials(secret: string, email?: string) {
  if (typeof window === 'undefined') return false
  const expectedSecret = process.env.NEXT_PUBLIC_OPERATOR_SECRET
  if (secret !== expectedSecret) return false
  
  sessionStorage.setItem('operator_secret', secret)
  if (email) sessionStorage.setItem('operator_email', email)
  return true
}

/**
 * Clear operator credentials (logout)
 */
export function clearOperatorCredentials() {
  if (typeof window === 'undefined') return
  sessionStorage.removeItem('operator_secret')
  sessionStorage.removeItem('operator_email')
  localStorage.removeItem('operator_secret')
  localStorage.removeItem('operator_email')
}
