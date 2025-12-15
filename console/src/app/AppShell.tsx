'use client'

import { useMemo, type ReactNode } from 'react'
import { usePathname } from 'next/navigation'
import Navigation from '@/components/Navigation'
import { AppShellV2 } from './AppShellV2'

const STATIC_PUBLIC_ROUTES = new Set(['/', '/login'])
const PUBLIC_ROUTE_PREFIXES = ['/auth', '/public']
// Admin routes have their own completely separate shell and authentication
// They must NOT be wrapped by the customer Navigation component
const ADMIN_ROUTE_PREFIX = '/admin'

function normalizePath(pathname: string | null): string {
  if (!pathname) return '/'
  if (pathname === '/') return '/'
  return pathname.replace(/\/$/, '') || '/'
}

function isPublicRoute(pathname: string | null): boolean {
  const normalized = normalizePath(pathname)
  if (STATIC_PUBLIC_ROUTES.has(normalized)) {
    return true
  }
  return PUBLIC_ROUTE_PREFIXES.some((prefix) => normalized.startsWith(prefix))
}

function isAdminRoute(pathname: string | null): boolean {
  const normalized = normalizePath(pathname)
  return normalized.startsWith(ADMIN_ROUTE_PREFIX)
}

export function AppShell({ children }: { children: ReactNode }) {
  const pathname = usePathname()
  const hideNavigation = useMemo(() => isPublicRoute(pathname), [pathname])
  // Admin routes are completely separate from customer console - no Navigation wrapper
  const isAdmin = useMemo(() => isAdminRoute(pathname), [pathname])

  // Admin panel has its own layout - bypass customer Navigation completely
  // This takes precedence over all other checks including v2 UI
  if (isAdmin) {
    return <>{children}</>
  }

  // Flagged cutover: when enabled, render the v2 AppShell
  const uiV2Enabled =
    process.env.NEXT_PUBLIC_UI_V2 === '1' || process.env.NEXT_PUBLIC_UI_V2 === 'true'

  if (uiV2Enabled) {
    return <AppShellV2>{children}</AppShellV2>
  }

  if (hideNavigation) {
    return <>{children}</>
  }

  return <Navigation>{children}</Navigation>
}
