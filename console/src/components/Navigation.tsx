'use client'

import Link from 'next/link'
import Image from 'next/image'
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
  LayoutGrid,
  ArrowLeftRight,
  Puzzle,
  TrendingUp,
  Eye,
  ShieldAlert,
  Wallet,
  Receipt,
  Link2,
  FileCheck,
  FileSearch,
  Bug,
  Settings,
  Shield,
  LogOut,
  Menu,
  X,
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
  { key: 'placements', label: 'Placements', href: '/placements', icon: LayoutGrid },
  { key: 'migration-studio', label: 'Migration Studio', labelKey: 'migrationStudio.nav', href: '/migration-studio', icon: ArrowLeftRight, featureFlag: 'migrationStudio' },
  { key: 'adapters', label: 'Adapters', href: '/adapters', icon: Puzzle },
  { key: 'analytics', label: 'Analytics', href: '/analytics', icon: TrendingUp },
  { key: 'transparency', label: 'Transparency', href: '/transparency/auctions', icon: Eye, featureFlag: 'transparency', minTier: 'growth' },
  { key: 'fraud', label: 'Fraud Detection', href: '/fraud', icon: ShieldAlert, minTier: 'growth' },
  { key: 'payouts', label: 'Payouts', href: '/payouts', icon: Wallet },
  { key: 'billing', label: 'Billing', href: '/billing/usage', icon: Receipt, featureFlag: 'billing' },
  { key: 'supply-chain', label: 'Supply Chain', href: '/supply-chain', icon: Link2 },
  { key: 'tool-supply-chain-status', label: 'Supply Chain Status', href: '/tools/supply-chain-status', icon: FileCheck },
  { key: 'tool-app-ads-inspector', label: 'app-ads.txt Inspector', href: '/tools/app-ads-inspector', icon: FileSearch },
  { key: 'tool-mediation-debugger', label: 'Mediation Debugger', href: '/tools/mediation-debugger', icon: Bug },
  { key: 'settings', label: 'Settings', href: '/settings', icon: Settings },
  { key: 'admin', label: 'Admin', href: '/admin/health', icon: Shield, roles: ['admin'] },
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

  const navSections = useMemo(() => {
    const byKey = new Map(navigation.map((item) => [item.key, item]))
    const build = (label: string, keys: string[]) => ({
      label,
      items: keys
        .map((key) => byKey.get(key))
        .filter(Boolean) as typeof navigation,
    })
    return [
      build('Workspace', ['dashboard', 'placements', 'migration-studio', 'adapters', 'analytics', 'transparency', 'fraud', 'payouts', 'billing']),
      build('Tools', ['supply-chain', 'tool-supply-chain-status', 'tool-app-ads-inspector', 'tool-mediation-debugger']),
      build('Settings', ['settings']),
      build('Admin', ['admin']),
    ].filter((section) => section.items.length > 0)
  }, [navigation])

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
    <div className="min-h-screen bg-[#f9fafb]">
      {/* Mobile menu button */}
      <div className="lg:hidden fixed top-0 left-0 right-0 z-50 bg-white border-b border-gray-100 px-4 py-3 flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="h-8 w-8 rounded-lg overflow-hidden ring-1 ring-gray-100 relative">
            <Image src="/logo.jpg" alt="ApexMediation Logo" fill className="object-cover" sizes="32px" priority />
          </div>
          <span className="text-sm font-semibold text-gray-900 tracking-tight">ApexMediation</span>
        </div>
        <button
          onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
          aria-label={mobileMenuOpen ? 'Close menu' : 'Open menu'}
          aria-expanded={mobileMenuOpen}
          className="p-2 hover:bg-gray-50 rounded-lg transition-colors"
        >
          {mobileMenuOpen ? (
            <X className="h-5 w-5 text-gray-500" aria-hidden={true} />
          ) : (
            <Menu className="h-5 w-5 text-gray-500" aria-hidden={true} />
          )}
        </button>
      </div>

      {/* Mobile menu overlay */}
      {mobileMenuOpen && (
        <div
          className="lg:hidden fixed inset-0 bg-black/40 z-40 backdrop-blur-sm"
          role="presentation"
          aria-hidden="true"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside
        className={`
          fixed top-0 left-0 bottom-0 w-[240px] bg-white border-r border-gray-100 z-40
          transition-transform duration-200 ease-in-out
          lg:translate-x-0
          ${mobileMenuOpen ? 'translate-x-0' : '-translate-x-full'}
        `}
      >
        <div className="flex flex-col h-full">
          {/* Logo */}
          <div className="px-5 py-6 border-b border-gray-100">
            <div className="flex items-center gap-3">
              <div className="h-10 w-10 rounded-xl overflow-hidden shadow-sm ring-1 ring-gray-100 flex-shrink-0 relative">
                <Image src="/logo.jpg" alt="ApexMediation Logo" fill className="object-cover" sizes="40px" priority />
              </div>
              <div className="min-w-0 flex-1">
                <h1 className="text-base font-semibold text-gray-900 tracking-tight">ApexMediation</h1>
                <p className="text-xs text-gray-500 font-medium tracking-tight">Publisher Console</p>
              </div>
            </div>
          </div>

          {/* Navigation */}
          <nav className="flex-1 px-3 py-5 space-y-4 overflow-y-auto" aria-label="Main navigation">
            {navSections.map((section) => (
              <div key={section.label} className="space-y-1">
                <div className="px-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-gray-400">
                  {section.label}
                </div>
                {section.items.map((item) => {
                  const isActive = pathname === item.href || pathname?.startsWith(`${item.href}/`)
                  const Icon = item.icon as any
                  return (
                    <Link
                      key={item.key}
                      href={item.href}
                      onClick={() => setMobileMenuOpen(false)}
                      className={`relative flex items-center gap-3 px-3 py-2.5 rounded-lg text-[13px] font-medium tracking-tight transition-colors duration-150
                        ${
                          isActive
                            ? 'bg-primary-50 text-primary-700 ring-1 ring-primary-100'
                            : 'text-gray-600 hover:bg-gray-50 hover:text-gray-900'
                        }
                      `}
                    >
                      {isActive && <span className="absolute left-0 top-1 bottom-1 w-0.5 rounded-full bg-primary-600" aria-hidden />}
                      <Icon className={`h-5 w-5 flex-shrink-0 ${isActive ? 'text-primary-700' : 'text-gray-400'}`} strokeWidth={1.75} aria-hidden={true} />
                      <span className="truncate text-[13px] leading-5">{item.name}</span>
                    </Link>
                  )
                })}
              </div>
            ))}
          </nav>

          {/* User menu */}
          <div className="p-4 border-t border-gray-100">
            <div className="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-gray-50 cursor-pointer group transition-all duration-150">
              <div className="h-8 w-8 rounded-full bg-gradient-to-br from-primary-500 to-primary-600 text-white flex items-center justify-center text-xs font-semibold">
                {user?.email?.charAt(0).toUpperCase() || 'U'}
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-[13px] font-medium text-gray-900 truncate tracking-tight">
                  {user?.email || 'User'}
                </p>
                  <p className="text-xs text-gray-400 truncate capitalize">{user?.role}</p>
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
                className="w-full mt-2 flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium text-gray-600 hover:bg-red-50 hover:text-red-600 transition-all duration-150 disabled:opacity-60 disabled:cursor-not-allowed"
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
      <div className="lg:pl-[240px]">
        <div className="pt-16 lg:pt-0 min-h-screen bg-[#f9fafb]">{children}</div>
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
    <div className="min-h-screen bg-[#f9fafb] animate-pulse" role="status" aria-live="polite">
      <div className="lg:hidden fixed top-0 left-0 right-0 z-50 bg-white border-b px-4 py-3 flex items-center justify-between">
        <div className="h-8 w-32 bg-gray-200 rounded" />
        <div className="h-8 w-8 bg-gray-200 rounded-full" />
      </div>
      <aside className="fixed top-0 left-0 bottom-0 w-[240px] bg-white border-r z-40 p-6 space-y-4 hidden lg:flex lg:flex-col">
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
