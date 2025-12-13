'use client'

import Link from 'next/link'
import { usePathname, useSelectedLayoutSegments, useRouter } from 'next/navigation'
import { useAdminGate, clearOperatorCredentials } from '@/lib/useAdminGate'

const ADMIN_NAV = [
  { name: 'Health', href: '/admin/health', segment: 'health' },
  { name: 'Infrastructure', href: '/admin/infrastructure', segment: 'infrastructure' },
  { name: 'Billing Ops', href: '/admin/billing', segment: 'billing' },
  { name: 'Audit Log', href: '/admin/audit', segment: 'audit' },
  { name: 'Sales Automation', href: '/admin/sales-automation', segment: 'sales-automation' },
  { name: 'Value Multipliers', href: '/admin/value-multipliers', segment: 'value-multipliers' },
]

export default function AdminLayout({ children }: { children: React.ReactNode }) {
  const router = useRouter()
  const pathname = usePathname()
  
  // Skip auth gate for the login page itself
  const isLoginPage = pathname === '/admin/login'
  
  const { isOperator, isLoading, user } = useAdminGate({ 
    unauthorizedRedirect: '/admin/login',
    disableRedirects: isLoginPage // Don't redirect when on login page
  })
  
  const segments = useSelectedLayoutSegments()
  const currentSegment = segments[0] ?? (pathname === '/admin' ? 'health' : undefined)

  const handleLogout = () => {
    clearOperatorCredentials()
    router.push('/admin/login')
  }

  // For the login page, just render the children without the gate
  if (isLoginPage) {
    return children
  }

  // Show loading state while checking operator access
  if (isLoading) {
    return (
      <div className="min-h-screen bg-gray-900 flex items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-primary-500 mx-auto"></div>
          <p className="mt-4 text-gray-400">Verifying operator access...</p>
        </div>
      </div>
    )
  }

  // Don't render anything if not an operator - redirect will happen via useAdminGate
  if (!isOperator) {
    return (
      <div className="min-h-screen bg-gray-900 flex items-center justify-center">
        <div className="text-center">
          <p className="text-red-400 text-lg">Access Denied</p>
          <p className="mt-2 text-gray-500 text-sm">Operator authentication required.</p>
          <Link 
            href="/admin/login"
            className="mt-4 inline-block text-primary-400 hover:text-primary-300"
          >
            Go to Operator Login →
          </Link>
        </div>
      </div>
    )
  }

  return (
    <div className="min-h-screen bg-[#f9fafb] px-6 py-10">
      <div className="mx-auto max-w-6xl space-y-6">
        <div className="flex items-start justify-between">
          <div className="space-y-2">
            <h1 className="text-2xl font-semibold tracking-tight text-gray-900">
              Operator Control Panel
            </h1>
            <p className="text-sm text-gray-600">
              System administration dashboard. Internal use only.
            </p>
          </div>
          <div className="flex items-center gap-4">
            {user?.email && (
              <span className="text-sm text-gray-500">
                {user.email}
              </span>
            )}
            <button
              onClick={handleLogout}
              className="text-sm text-gray-600 hover:text-red-600 transition-colors"
            >
              Logout
            </button>
          </div>
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
