'use client'

import Link from 'next/link'
import { usePathname, useSelectedLayoutSegments } from 'next/navigation'
import { useAdminGate } from '@/lib/useAdminGate'

const ADMIN_NAV = [
  { name: 'Health', href: '/admin/health', segment: 'health' },
  { name: 'Infrastructure', href: '/admin/infrastructure', segment: 'infrastructure' },
  { name: 'Billing Ops', href: '/admin/billing', segment: 'billing' },
  { name: 'Audit Log', href: '/admin/audit', segment: 'audit' },
  { name: 'Sales Automation', href: '/admin/sales-automation', segment: 'sales-automation' },
  { name: 'Value Multipliers', href: '/admin/value-multipliers', segment: 'value-multipliers' },
]

export default function AdminLayout({ children }: { children: React.ReactNode }) {
  useAdminGate()
  const pathname = usePathname()
  const segments = useSelectedLayoutSegments()
  const currentSegment = segments[0] ?? (pathname === '/admin' ? 'health' : undefined)

  return (
    <div className="min-h-screen bg-[#f9fafb] px-6 py-10">
      <div className="mx-auto max-w-6xl space-y-6">
        <div className="space-y-2">
          <h1 className="text-2xl font-semibold tracking-tight text-gray-900">Admin</h1>
          <p className="text-sm text-gray-600">Operator controls and readouts.</p>
        </div>

        <div className="rounded-xl border border-gray-200 bg-white p-2">
          <nav className="flex gap-1" aria-label="Admin navigation">
            {ADMIN_NAV.map((tab) => {
              const active = currentSegment === tab.segment
              return (
                <Link
                  key={tab.href}
                  href={tab.href}
                  className={`relative flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium transition-colors ${
                    active ? 'bg-primary-50 text-primary-700 ring-1 ring-primary-100' : 'text-gray-700 hover:bg-gray-50 hover:text-gray-900'
                  }`}
                  aria-current={active ? 'page' : undefined}
                >
                  {active && <span className="absolute left-0 top-1 bottom-1 w-0.5 rounded-full bg-primary-600" aria-hidden />}
                  {tab.name}
                </Link>
              )
            })}
          </nav>
        </div>

        <div>{children}</div>
      </div>
    </div>
  )
}
