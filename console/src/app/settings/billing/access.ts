import type { Role } from '@/lib/rbac'
import type { SessionUser } from '@/lib/useSession'

export const BILLING_ALLOWED_ROLES: Role[] = ['admin', 'publisher']
export const BILLING_ALLOWED_PERMISSIONS = ['billing:view', 'billing:manage']

export const BILLING_FEATURE_FALLBACK = {
  billing: process.env.NEXT_PUBLIC_BILLING_ENABLED === 'true',
}

export function canAccessBilling(user: SessionUser | null): boolean {
  if (!user) return false
  const role = (user.role ?? 'readonly') as Role
  const hasRoleAccess = BILLING_ALLOWED_ROLES.includes(role)
  const hasPermission = (user.permissions ?? []).some((permission) =>
    BILLING_ALLOWED_PERMISSIONS.includes(permission)
  )
  return hasRoleAccess || hasPermission
}
