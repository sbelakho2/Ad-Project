'use client'

import Link from 'next/link'
import { signOut } from 'next-auth/react'
import { usePathname } from 'next/navigation'
import { useEffect, useMemo, useState } from 'react'
import { useSession as useCookieSession } from '@/lib/useSession'
import { useFeatures } from '@/lib/useFeatures'
import { t } from '@/i18n'
import type { Role } from '@/lib/rbac'
import { usePlanAccess } from '@/lib/usePlanAccess'
import {
  LayoutDashboard,
  Layout,
  Layers,
  Settings,
  DollarSign,
  ShieldAlert,
  BarChart3,
  LogOut,
  Menu,
  X,
  ShieldCheck,
  CreditCard,
  GitCompare,
  Wrench,
} from 'lucide-react'

type FeatureFlagKey = 'transparency' | 'billing' | 'migrationStudio'
type NavBlueprint = {
  key: string
  label: string
  labelKey?: string
  href: string
  icon: typeof LayoutDashboard
  featureFlag?: FeatureFlagKey
  roles?: Role[]
  minTier?: 'starter' | 'growth' | 'scale' | 'enterprise'
}

const NAV_BLUEPRINT: NavBlueprint[] = [
  { key: 'dashboard', label: 'Dashboard', href: '/dashboard', icon: LayoutDashboard },
  { key: 'placements', label: 'Placements', href: '/placements', icon: Layout },
  { key: 'migration-studio', label: 'Migration Studio', labelKey: 'migrationStudio.nav', href: '/migration-studio', icon: GitCompare, featureFlag: 'migrationStudio' },
  { key: 'adapters', label: 'Adapters', href: '/adapters', icon: Layers },
  { key: 'analytics', label: 'Analytics', href: '/analytics', icon: BarChart3 },
  { key: 'transparency', label: 'Transparency', href: '/transparency/auctions', icon: ShieldCheck, featureFlag: 'transparency', minTier: 'growth' },
  { key: 'fraud', label: 'Fraud Detection', href: '/fraud', icon: ShieldAlert, minTier: 'growth' },
  { key: 'payouts', label: 'Payouts', href: '/payouts', icon: DollarSign },
  { key: 'billing', label: 'Billing', href: '/billing/usage', icon: CreditCard, featureFlag: 'billing' },
  // Developer tools (always visible to authenticated users)
  { key: 'supply-chain', label: 'Supply Chain', href: '/supply-chain', icon: ShieldCheck },
  { key: 'tool-supply-chain-status', label: 'Supply Chain Status', href: '/tools/supply-chain-status', icon: ShieldCheck },
  { key: 'tool-app-ads-inspector', label: 'app-ads.txt Inspector', href: '/tools/app-ads-inspector', icon: Wrench },
  { key: 'tool-mediation-debugger', label: 'Mediation Debugger', href: '/tools/mediation-debugger', icon: BarChart3 },
  { key: 'settings', label: 'Settings', href: '/settings', icon: Settings },
  { key: 'admin', label: 'Admin', href: '/admin/health', icon: Settings, roles: ['admin'] },
]

const NAV_SKELETON_KEYS = ['nav-skeleton-1', 'nav-skeleton-2', 'nav-skeleton-3', 'nav-skeleton-4', 'nav-skeleton-5', 'nav-skeleton-6']

export default function Navigation({ children }: { children: React.ReactNode }) {
  const pathname = usePathname()
  const { user, isLoading: sessionLoading } = useCookieSession()
  const bypassLoadingGuards = Boolean(process.env.NEXT_PUBLIC_E2E_SESSION)
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)
  const [signingOut, setSigningOut] = useState(false)
  const { plan, isLoading: planLoading } = usePlanAccess()

  // Prefer runtime feature flags from API, with build-time env as SSR fallback
  const featureFallbacks = useMemo(
    () => ({
      transparency: process.env.NEXT_PUBLIC_TRANSPARENCY_ENABLED === 'true',
      billing: process.env.NEXT_PUBLIC_BILLING_ENABLED === 'true',
      migrationStudio: process.env.NEXT_PUBLIC_MIGRATION_STUDIO_ENABLED === 'true',
    }),
    []
  )

  const { features, loading: featuresLoading, refresh: refreshFeatures } = useFeatures({ fallback: featureFallbacks })

  const resolvedFeatures = useMemo(() => {
    return {
      transparency: featureFallbacks.transparency,
      billing: featureFallbacks.billing,
      migrationStudio: featureFallbacks.migrationStudio,
      ...(features || {}),
    }
  }, [featureFallbacks.billing, featureFallbacks.migrationStudio, featureFallbacks.transparency, features])

  useEffect(() => {
    if (typeof window === 'undefined') return
    const handleVisibility = () => {
      if (document.hidden) return
      refreshFeatures()
    }
    window.addEventListener('visibilitychange', handleVisibility)
    window.addEventListener('focus', handleVisibility)
    const intervalId = window.setInterval(handleVisibility, 60 * 1000)
    return () => {
      window.removeEventListener('visibilitychange', handleVisibility)
      window.removeEventListener('focus', handleVisibility)
      window.clearInterval(intervalId)
    }
  }, [refreshFeatures])

  const navigation = useMemo(() => {
    const role: Role | undefined = user?.role as Role | undefined
    return NAV_BLUEPRINT.filter((item) => {
      if (item.roles && (!role || !item.roles.includes(role))) {
        return false
      }
      if (item.featureFlag && !resolvedFeatures[item.featureFlag]) {
        return false
      }
      if (item.minTier && plan) {
        const tierOrder = ['starter', 'growth', 'scale', 'enterprise'] as const
        const currentIdx = tierOrder.indexOf(plan as any)
        const minIdx = tierOrder.indexOf(item.minTier as any)
        if (currentIdx > -1 && minIdx > -1 && currentIdx < minIdx) {
          return false
        }
      }
      return true
    }).map((item) => ({
      ...item,
      name: resolveNavLabel(item),
    }))
  }, [plan, resolvedFeatures, user?.role])

  // Don't show navigation on login page
  if (!pathname || pathname === '/login' || pathname === '/') return <>{children}</>

  if (!bypassLoadingGuards && (sessionLoading || featuresLoading || planLoading)) {
    return <NavigationSkeleton />
  }

  // If session missing (logging out / unauthorized), defer to page-level redirects without flashing nav
  if (!user) {
    return <>{children}</>
  }

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Mobile menu button */}
      <div className="lg:hidden fixed top-0 left-0 right-0 z-50 bg-white border-b px-4 py-3 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <div className="h-8 w-8 rounded-lg bg-primary-600 flex items-center justify-center">
            <span className="text-white font-bold text-sm">A</span>
          </div>
          <span className="font-semibold text-gray-900">ApexMediation</span>
        </div>
        <button
          onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
          aria-label={mobileMenuOpen ? 'Close menu' : 'Open menu'}
          aria-expanded={mobileMenuOpen}
          className="p-2 hover:bg-gray-100 rounded-lg"
        >
          {mobileMenuOpen ? (
            <X className="h-6 w-6 text-gray-600" aria-hidden={true} />
          ) : (
            <Menu className="h-6 w-6 text-gray-600" aria-hidden={true} />
          )}
        </button>
      </div>

      {/* Mobile menu overlay */}
      {mobileMenuOpen && (
        <div
          className="lg:hidden fixed inset-0 bg-black/50 z-40"
          role="presentation"
          aria-hidden="true"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside
        className={`
          fixed top-0 left-0 bottom-0 w-64 bg-white border-r z-40
          transition-transform duration-200 ease-in-out
          lg:translate-x-0
          ${mobileMenuOpen ? 'translate-x-0' : '-translate-x-full'}
        `}
      >
        <div className="flex flex-col h-full">
          {/* Logo */}
          <div className="p-6 border-b">
            <div className="flex items-center gap-3">
              <div className="h-10 w-10 rounded-lg bg-primary-600 flex items-center justify-center overflow-hidden">
                <img src="/logo.jpg" alt="ApexMediation Logo" className="h-full w-full object-cover" />
              </div>
              <div>
                <h1 className="font-semibold text-gray-900">ApexMediation</h1>
                <p className="text-xs text-gray-600">Publisher Console</p>
              </div>
            </div>
          </div>

          {/* Navigation */}
          <nav className="flex-1 p-4 space-y-1 overflow-y-auto">
            {navigation?.map((item) => {
              const isActive = pathname === item.href || pathname?.startsWith(`${item.href}/`)
              const Icon = item.icon as any
              return (
                <Link
                  key={item.key}
                  href={item.href}
                  onClick={() => setMobileMenuOpen(false)}
                  className={`
                    flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium transition
                    ${
                      isActive
                        ? 'bg-primary-50 text-primary-700'
                        : 'text-gray-700 hover:bg-gray-50 hover:text-gray-900'
                    }
                  `}
                >
                  <Icon className="h-5 w-5" aria-hidden={true} />
                  {item.name}
                </Link>
              )
            })}
          </nav>

          {/* User menu */}
          <div className="p-4 border-t">
            <div className="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-gray-50 cursor-pointer group">
              <div className="h-8 w-8 rounded-full bg-primary-100 text-primary-700 flex items-center justify-center text-sm font-semibold">
                {user?.email?.charAt(0).toUpperCase() || 'U'}
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-gray-900 truncate">
                  {user?.email || 'User'}
                </p>
                  <p className="text-xs text-gray-600 truncate">{user?.role}</p>
                </div>
              </div>
              <button
                onClick={async () => {
                  if (signingOut) return
                  setSigningOut(true)
                  try {
                    await signOut({ callbackUrl: '/login' })
                  } finally {
                    setSigningOut(false)
                  }
                }}
                disabled={signingOut}
                className="w-full mt-2 flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium text-gray-700 hover:bg-gray-50 hover:text-gray-900 transition disabled:opacity-60 disabled:cursor-not-allowed"
                aria-busy={signingOut}
              >
                <LogOut className="h-5 w-5" aria-hidden={true} />
                <span className="flex-1 text-left" aria-live="polite">
                  {signingOut ? 'Signing Out…' : 'Sign Out'}
                </span>
              </button>
          </div>
        </div>
      </aside>

      {/* Main content */}
      <div className="lg:pl-64">
        <div className="pt-16 lg:pt-0">{children}</div>
      </div>
    </div>
  )
}

function resolveNavLabel(item: NavBlueprint): string {
  if (item.labelKey) {
    const translated = t(item.labelKey)
    if (translated && translated !== item.labelKey) {
      return translated
    }
  }
  return item.label
}

function NavigationSkeleton() {
  return (
    <div className="min-h-screen bg-gray-50 animate-pulse" role="status" aria-live="polite">
      <div className="lg:hidden fixed top-0 left-0 right-0 z-50 bg-white border-b px-4 py-3 flex items-center justify-between">
        <div className="h-8 w-32 bg-gray-200 rounded" />
        <div className="h-8 w-8 bg-gray-200 rounded-full" />
      </div>
      <aside className="fixed top-0 left-0 bottom-0 w-64 bg-white border-r z-40 p-6 space-y-4 hidden lg:flex lg:flex-col">
        <div className="h-10 w-full bg-gray-200 rounded" />
        <div className="space-y-2">
          {NAV_SKELETON_KEYS.map((key) => (
            <div key={key} className="h-9 w-full bg-gray-100 rounded" />
          ))}
        </div>
        <div className="mt-auto space-y-2">
          <div className="h-8 bg-gray-100 rounded" />
          <div className="h-10 bg-gray-100 rounded" />
        </div>
      </aside>
    </div>
  )
}
