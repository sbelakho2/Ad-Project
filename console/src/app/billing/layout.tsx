'use client'

import Link from 'next/link'
import { useEffect, useMemo } from 'react'
import { usePathname, useRouter } from 'next/navigation'
import { CreditCard, FileText, TrendingUp, Settings } from 'lucide-react'
import { useSession } from '@/lib/useSession'
import { useFeatures } from '@/lib/useFeatures'
import {
  BILLING_ALLOWED_PERMISSIONS,
  BILLING_ALLOWED_ROLES,
  BILLING_FEATURE_FALLBACK,
  canAccessBilling as canUserAccessBilling,
} from './access'

type BillingNavItem = {
  name: string
  href: string
  icon: typeof TrendingUp
  featureFlag?: keyof NonNullable<ReturnType<typeof useFeatures>['features']>
  permission?: string
}

const BILLING_NAV: BillingNavItem[] = [
  { name: 'Usage', href: '/billing/usage', icon: TrendingUp },
  { name: 'Invoices', href: '/billing/invoices', icon: FileText },
  { name: 'Settings', href: '/billing/settings', icon: Settings },
]


export default function BillingLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname()
  const router = useRouter()
  const { user, isLoading: sessionLoading } = useSession()
  const { features, loading: featureLoading } = useFeatures({ fallback: BILLING_FEATURE_FALLBACK })

  const hasBillingFeature = features?.billing ?? BILLING_FEATURE_FALLBACK.billing ?? false
  const canAccessBilling = canUserAccessBilling(user)

  useEffect(() => {
    if (!sessionLoading && !user) {
      router.replace('/login')
    }
  }, [sessionLoading, user, router])

  useEffect(() => {
    if (sessionLoading || featureLoading) return
    if (!hasBillingFeature || !canAccessBilling) {
      router.replace('/403')
    }
  }, [sessionLoading, featureLoading, hasBillingFeature, canAccessBilling, router])

  const navItems = useMemo(() => {
    if (!hasBillingFeature || !canAccessBilling) return []
    return BILLING_NAV.filter((item) => {
      if (item.featureFlag && features && features[item.featureFlag] === false) {
        return false
      }
      if (item.permission && !(user?.permissions ?? []).includes(item.permission)) {
        return false
      }
      return true
    })
  }, [canAccessBilling, features, hasBillingFeature, user?.permissions])

  if (sessionLoading || featureLoading) {
    return (
      <div className="bg-white border rounded-lg p-6 text-gray-700" role="status">
        Loading billing workspace…
      </div>
    )
  }

  if (!hasBillingFeature) {
    return (
      <div className="bg-white border rounded-lg p-6 text-gray-700" role="alert">
        Billing is disabled for this environment. Contact support if you believe this is an error.
      </div>
    )
  }

  if (!canAccessBilling) {
    return (
      <div className="bg-white border rounded-lg p-6 text-gray-700" role="alert">
        You do not have permission to manage billing. Redirecting…
      </div>
    )
  }

  return (
    <div>
      {/* Billing Sub-Navigation */}
      <div className="border-b border-gray-200 bg-white" role="navigation" aria-label="Billing sections">
        <div className="px-8 py-5">
          <div className="max-w-6xl mx-auto flex flex-col gap-4">
            <div className="flex items-center justify-between gap-4">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary-100">
                  <CreditCard className="h-5 w-5 text-primary-600" />
                </div>
                <div className="leading-tight">
                  <p className="text-sm font-medium text-gray-500">Billing</p>
                  <h1 className="text-2xl font-semibold tracking-tight text-gray-900">Usage & invoices</h1>
                </div>
              </div>
            </div>
            <nav className="flex items-center gap-2 border-t border-gray-200 pt-2" aria-label="Billing tabs">
              {navItems.map((item) => {
                const Icon = item.icon
                const isActive = pathname === item.href || pathname?.startsWith(`${item.href}/`)
                return (
                  <Link
                    key={item.name}
                    href={item.href}
                    aria-current={isActive ? 'page' : undefined}
                    className={`relative flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-gray-700 transition-colors hover:text-gray-900 ${
                      isActive ? 'bg-primary-50 text-primary-700' : 'hover:bg-gray-50'
                    }`}
                  >
                    <Icon className="h-4 w-4" />
                    <span className="tracking-tight">{item.name}</span>
                    {isActive && (
                      <span className="absolute left-0 top-1 bottom-1 w-0.5 rounded-full bg-primary-600" aria-hidden="true" />
                    )}
                  </Link>
                )
              })}
              {navItems.length === 0 && (
                <p className="text-xs text-gray-500" role="status">
                  No billing tools are enabled for your account yet.
                </p>
              )}
            </nav>
          </div>
        </div>
      </div>

      {/* Page Content */}
      {children}
    </div>
  )
}
