'use client'

import Link from 'next/link'
import Image from 'next/image'
import { usePathname } from 'next/navigation'
import type { ReactNode } from 'react'
import { ToastProvider } from '@/ui-v2/hooks/useToast'
import { Toaster } from '@/ui-v2/components/Toaster'
import { Breadcrumbs } from '@/ui-v2/components/Breadcrumbs'
import { CommandPalette } from '@/ui-v2/components/CommandPalette'
import { useMemo } from 'react'
import { useSession as useCookieSession } from '@/lib/useSession'
import { useFeatures } from '@/lib/useFeatures'

function isActive(pathname: string | null, href: string) {
  if (!pathname) return false
  if (href === '/') return pathname === '/'
  return pathname === href || pathname.startsWith(href + '/')
}

function NavLink({ href, children }: { href: string; children: ReactNode }) {
  const pathname = usePathname()
  const active = isActive(pathname, href)
  const base = 'relative flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium transition-colors'
  const activeCls = 'bg-primary-50 text-primary-700 ring-1 ring-primary-100'
  const idleCls = 'text-zinc-700 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800'
  return (
    <Link href={href} className={[base, active ? activeCls : idleCls].join(' ')}>
      {active && <span className="absolute left-0 top-1 bottom-1 w-0.5 rounded-full bg-primary-600" aria-hidden />}
      {children}
    </Link>
  )
}

export function AppShellV2({ children }: { children: ReactNode }) {
  // Runtime feature flags and role-aware navigation (lightweight)
  const { user } = useCookieSession()
  const featureFallbacks = useMemo(
    () => ({
      transparency: process.env.NEXT_PUBLIC_TRANSPARENCY_ENABLED === 'true',
      billing: process.env.NEXT_PUBLIC_BILLING_ENABLED === 'true',
      migrationStudio: process.env.NEXT_PUBLIC_MIGRATION_STUDIO_ENABLED === 'true',
    }),
    []
  )
  const { features } = useFeatures({ fallback: featureFallbacks })
  const resolved = useMemo(
    () =>
      ({
        transparency: featureFallbacks.transparency,
        billing: featureFallbacks.billing,
        migrationStudio: featureFallbacks.migrationStudio,
        ...(features || {}),
      }) as Record<string, boolean>,
    [featureFallbacks, features]
  )

  // NOTE: Admin panel is for OPERATORS ONLY - not exposed in customer navigation
  // Operators access directly via /admin/login

  const navLinks = useMemo(() => {
    const items: { href: string; label: string; feature?: keyof typeof resolved }[] = [
      { href: '/dashboard', label: 'Dashboard' },
      { href: '/settings/billing/usage', label: 'Billing', feature: 'billing' as any },
      { href: '/transparency/auctions', label: 'Transparency', feature: 'transparency' as any },
      { href: '/settings', label: 'Settings' },
      // Admin link removed - operator cockpit is not for customers
    ]
    return items.filter((it) => {
      if (it.feature && !resolved[it.feature]) return false
      return true
    })
  }, [resolved])

  return (
    <ToastProvider>
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-[100] focus:rounded-md focus:bg-zinc-900 focus:px-3 focus:py-2 focus:text-white dark:focus:bg-zinc-100 dark:focus:text-zinc-900"
      >
        Skip to content
      </a>
      <div className="min-h-screen bg-[#f9fafb] text-zinc-900 dark:bg-zinc-950 dark:text-zinc-50">
        <header className="sticky top-0 z-40 w-full border-b border-zinc-200 bg-white/90 backdrop-blur">
          <div className="mx-auto flex h-14 max-w-7xl items-center justify-between px-4 sm:px-6 lg:px-8">
            <div className="flex items-center gap-2">
              <Link href="/" className="inline-flex items-center gap-2" aria-label="ApexMediation Home">
                {/* Branding via repository root logo served from /api/brand/logo */}
                <Image
                  src="/api/brand/logo"
                  alt="ApexMediation"
                  width={28}
                  height={28}
                  className="h-7 w-7 rounded-sm shadow-sm"
                  priority
                />
                <span className="text-sm font-semibold tracking-tight text-zinc-800">ApexMediation</span>
              </Link>
              <span className="rounded bg-emerald-600/10 px-2 py-0.5 text-xs font-medium text-emerald-700">Console v2</span>
            </div>
            <nav className="hidden items-center gap-1 md:flex">
              {navLinks.map((l) => (
                <NavLink key={l.href} href={l.href}>{l.label}</NavLink>
              ))}
            </nav>
          </div>
        </header>
        <main id="main-content" className="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8">
          <Breadcrumbs />
          {children}
        </main>
        <Toaster />
        <CommandPalette />
      </div>
    </ToastProvider>
  )
}

export default AppShellV2
