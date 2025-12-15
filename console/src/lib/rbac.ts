/**
 * CUSTOMER RBAC SYSTEM
 * 
 * This module defines role-based access control for CUSTOMER console users.
 * 
 * CRITICAL SEPARATION:
 * - Customer roles (admin, publisher, readonly) control access to customer features
 * - Customer 'admin' role is NOT the same as system operator access
 * - Operator access uses a completely separate authentication system (see useAdminGate.ts)
 * 
 * Role hierarchy for customer console:
 * - admin: Full access to organization settings, billing, team management
 * - publisher: Can manage placements, view analytics, access tools
 * - readonly: View-only access to dashboards and reports
 * 
 * NOTE: None of these roles grant access to the Operator Control Panel (/admin/*)
 */
export type Role = 'admin' | 'publisher' | 'readonly'

/**
 * Type guard to check if a value is a valid customer Role
 */
export function isCustomerRole(value: unknown): value is Role {
  return value === 'admin' || value === 'publisher' || value === 'readonly'
}

export class RbacError extends Error {
  status: number
  reason: string

  constructor(message: string, reason: string) {
    super(message)
    this.name = 'RbacError'
    this.status = 403
    this.reason = reason
  }
}

export function hasRole(userRole: Role | null | undefined, allowed: Role[]): boolean {
  if (!userRole) return false
  return allowed.includes(userRole)
}

export function assertRole(userRole: Role | null | undefined, allowed: Role[], message?: string) {
  if (!hasRole(userRole, allowed)) {
    const reason = `requires-role:${allowed.join('|')}`
    throw new RbacError(message || 'Forbidden', reason)
  }
}
