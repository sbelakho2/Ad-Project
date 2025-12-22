import { getServerSession } from 'next-auth'
import { redirect } from 'next/navigation'
import { authOptions } from '@/lib/auth/options'
import { hasRole, type Role } from '@/lib/rbac'

/**
 * RBAC-protected layout for all /settings/* routes
 * 
 * Only 'admin' and 'publisher' roles can access settings.
 * 'readonly' users are redirected to /403.
 * 
 * This layout provides a single enforcement point for all nested settings pages,
 * preventing direct URL access bypass of the main settings page RBAC check.
 */

const SETTINGS_ROUTE = '/settings'
const ALLOWED_ROLES: Role[] = ['admin', 'publisher']

const IS_E2E_MOCK_MODE =
  process.env.NODE_ENV !== 'production' &&
  process.env.NEXT_PUBLIC_USE_MOCK_API === 'true' &&
  Boolean(process.env.NEXT_PUBLIC_E2E_SESSION)

export default async function SettingsLayout({
  children,
}: {
  children: React.ReactNode
}) {
  // In local mock-mode E2E runs we don't have a real NextAuth server session cookie.
  // Allow the settings subtree to render so Playwright can validate UI wiring deterministically.
  if (IS_E2E_MOCK_MODE) {
    return <>{children}</>
  }

  const session = await getServerSession(authOptions)

  // Redirect to login if not authenticated
  if (!session?.user) {
    redirect(`/login?next=${encodeURIComponent(SETTINGS_ROUTE)}`)
  }

  const role = (session.user as { role?: Role | null })?.role ?? null
  const publisherId = (session.user as { publisherId?: string | null })?.publisherId ?? null

  // RBAC enforcement: only admin and publisher roles can access settings
  if (!hasRole(role, ALLOWED_ROLES)) {
    redirect('/403')
  }

  // Publisher ID is required for all settings operations
  if (!publisherId) {
    redirect('/403')
  }

  return <>{children}</>
}
