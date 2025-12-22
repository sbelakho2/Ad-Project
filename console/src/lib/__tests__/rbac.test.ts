/**
 * RBAC Edge Case Tests
 * 
 * Tests all RBAC roles and permission edge cases to ensure security.
 * 
 * Roles tested:
 * - admin: Full access to organization settings, billing, team
 * - publisher: Can manage placements, view analytics, access tools
 * - readonly: View-only access to dashboards
 * 
 * Edge cases:
 * - Direct URL access bypass attempts
 * - Role manipulation in session
 * - Missing or null roles
 * - Permission inheritance
 */

import { hasRole, assertRole, isCustomerRole, RbacError, type Role } from '@/lib/rbac'
import { canAccessBilling } from '@/app/settings/billing/access'

describe('RBAC Core Functions', () => {
  describe('isCustomerRole', () => {
    it('returns true for valid roles', () => {
      expect(isCustomerRole('admin')).toBe(true)
      expect(isCustomerRole('publisher')).toBe(true)
      expect(isCustomerRole('readonly')).toBe(true)
    })

    it('returns false for invalid roles', () => {
      expect(isCustomerRole('operator')).toBe(false)
      expect(isCustomerRole('superadmin')).toBe(false)
      expect(isCustomerRole('')).toBe(false)
      expect(isCustomerRole(null)).toBe(false)
      expect(isCustomerRole(undefined)).toBe(false)
      expect(isCustomerRole(123)).toBe(false)
      expect(isCustomerRole({})).toBe(false)
    })

    it('prevents privilege escalation via typosquatting', () => {
      expect(isCustomerRole('Admin')).toBe(false)
      expect(isCustomerRole('ADMIN')).toBe(false)
      expect(isCustomerRole('admin ')).toBe(false)
      expect(isCustomerRole(' admin')).toBe(false)
    })
  })

  describe('hasRole', () => {
    it('returns true when user role is in allowed list', () => {
      expect(hasRole('admin', ['admin', 'publisher'])).toBe(true)
      expect(hasRole('publisher', ['admin', 'publisher'])).toBe(true)
      expect(hasRole('readonly', ['readonly'])).toBe(true)
    })

    it('returns false when user role is NOT in allowed list', () => {
      expect(hasRole('readonly', ['admin', 'publisher'])).toBe(false)
      expect(hasRole('publisher', ['admin'])).toBe(false)
    })

    it('handles null/undefined roles safely', () => {
      expect(hasRole(null, ['admin', 'publisher'])).toBe(false)
      expect(hasRole(undefined, ['admin', 'publisher'])).toBe(false)
    })

    it('handles empty allowed list', () => {
      expect(hasRole('admin', [])).toBe(false)
    })
  })

  describe('assertRole', () => {
    it('does not throw when role is allowed', () => {
      expect(() => assertRole('admin', ['admin', 'publisher'])).not.toThrow()
      expect(() => assertRole('publisher', ['admin', 'publisher'])).not.toThrow()
    })

    it('throws RbacError when role is not allowed', () => {
      expect(() => assertRole('readonly', ['admin'])).toThrow(RbacError)
    })

    it('throws RbacError with correct status for 403', () => {
      try {
        assertRole('readonly', ['admin'])
        fail('Expected to throw')
      } catch (e) {
        expect(e).toBeInstanceOf(RbacError)
        expect((e as RbacError).status).toBe(403)
        expect((e as RbacError).reason).toBe('requires-role:admin')
      }
    })

    it('throws for null/undefined roles', () => {
      expect(() => assertRole(null, ['admin'])).toThrow(RbacError)
      expect(() => assertRole(undefined, ['admin'])).toThrow(RbacError)
    })

    it('includes all required roles in error reason', () => {
      try {
        assertRole('readonly', ['admin', 'publisher'])
        fail('Expected to throw')
      } catch (e) {
        expect((e as RbacError).reason).toBe('requires-role:admin|publisher')
      }
    })

    it('uses custom message when provided', () => {
      try {
        assertRole('readonly', ['admin'], 'Custom forbidden message')
        fail('Expected to throw')
      } catch (e) {
        expect((e as Error).message).toBe('Custom forbidden message')
      }
    })
  })
})

describe('Billing Access Control', () => {
  it('allows admin role', () => {
    expect(
      canAccessBilling({
        userId: 'user_test',
        publisherId: 'pub_test',
        email: 'test@example.com',
        role: 'admin',
        permissions: [],
      })
    ).toBe(true)
  })

  it('allows publisher role', () => {
    expect(
      canAccessBilling({
        userId: 'user_test',
        publisherId: 'pub_test',
        email: 'test@example.com',
        role: 'publisher',
        permissions: [],
      })
    ).toBe(true)
  })

  it('denies readonly role without billing permissions', () => {
    expect(
      canAccessBilling({
        userId: 'user_test',
        publisherId: 'pub_test',
        email: 'test@example.com',
        role: 'readonly',
        permissions: [],
      })
    ).toBe(false)
  })

  it('allows readonly role with billing:view permission', () => {
    expect(
      canAccessBilling({
        userId: 'user_test',
        publisherId: 'pub_test',
        email: 'test@example.com',
        role: 'readonly',
        permissions: ['billing:view'],
      })
    ).toBe(true)
  })

  it('allows readonly role with billing:manage permission', () => {
    expect(
      canAccessBilling({
        userId: 'user_test',
        publisherId: 'pub_test',
        email: 'test@example.com',
        role: 'readonly',
        permissions: ['billing:manage'],
      })
    ).toBe(true)
  })

  it('denies null user', () => {
    expect(canAccessBilling(null)).toBe(false)
  })

  it('defaults to readonly when role is missing', () => {
    // @ts-expect-error - Testing runtime behavior with missing role
    expect(canAccessBilling({ email: 'test@example.com' })).toBe(false)
  })

  it('handles undefined permissions array', () => {
    expect(
      canAccessBilling({
        userId: 'user_test',
        publisherId: 'pub_test',
        email: 'test@example.com',
        role: 'admin',
        permissions: undefined,
      })
    ).toBe(true)
  })
})

describe('Role Hierarchy Edge Cases', () => {
  const roles: Role[] = ['admin', 'publisher', 'readonly']

  it('admin can access all admin-allowed routes', () => {
    expect(hasRole('admin', ['admin'])).toBe(true)
    expect(hasRole('admin', ['admin', 'publisher'])).toBe(true)
    expect(hasRole('admin', ['admin', 'publisher', 'readonly'])).toBe(true)
  })

  it('publisher cannot access admin-only routes', () => {
    expect(hasRole('publisher', ['admin'])).toBe(false)
  })

  it('readonly cannot access admin or publisher routes', () => {
    expect(hasRole('readonly', ['admin'])).toBe(false)
    expect(hasRole('readonly', ['admin', 'publisher'])).toBe(false)
  })

  it('role strings are case-sensitive', () => {
    // Ensure no accidental matches with different casing
    expect(hasRole('Admin' as Role, ['admin'])).toBe(false)
    expect(hasRole('PUBLISHER' as Role, ['publisher'])).toBe(false)
  })
})

describe('Security Edge Cases', () => {
  it('prevents object injection via role field', () => {
    // Attacker might try to pass object with toString
    const maliciousRole = { toString: () => 'admin' } as unknown as Role
    expect(hasRole(maliciousRole, ['admin'])).toBe(false)
  })

  it('prevents array injection via role field', () => {
    const maliciousRole = ['admin'] as unknown as Role
    expect(hasRole(maliciousRole, ['admin'])).toBe(false)
  })

  it('handles prototype pollution attempts', () => {
    const pollutedRole = Object.create({ __proto__: { includes: () => true } })
    pollutedRole.value = 'readonly'
    expect(hasRole(pollutedRole as Role, ['admin'])).toBe(false)
  })
})
