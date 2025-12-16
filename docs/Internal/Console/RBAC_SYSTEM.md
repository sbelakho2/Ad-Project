# Role-Based Access Control (RBAC) System

## Overview

The console implements a three-tier RBAC system for customer organizations, completely separate from the operator control plane.

## Role Definitions

| Role | Description | Primary Use Case |
|------|-------------|------------------|
| `admin` | Full organization access | Business owners, account managers |
| `publisher` | Operational access | Ad ops, developers, analysts |
| `readonly` | View-only access | Stakeholders, auditors, clients |

## Permission Matrix

### Core Features

| Feature | `admin` | `publisher` | `readonly` |
|---------|---------|-------------|------------|
| Dashboard | ✅ | ✅ | ✅ |
| Analytics | ✅ | ✅ | ✅ |
| Placements | ✅ Create/Edit | ✅ Create/Edit | ❌ View only |
| A/B Tests | ✅ | ✅ | ❌ |
| Migration Studio | ✅ | ✅ | ❌ |

### Settings (Protected by Layout)

| Feature | `admin` | `publisher` | `readonly` |
|---------|---------|-------------|------------|
| Settings Index | ✅ | ✅ | ❌ 403 |
| Security & 2FA | ✅ | ✅ | ❌ 403 |
| API Keys | ✅ | ✅ | ❌ 403 |
| Fraud Protection | ✅ | ✅ | ❌ 403 |
| Payouts | ✅ | ✅ | ❌ 403 |
| Team Access | ✅ | ❌ | ❌ 403 |
| Notifications | ✅ | ✅ | ❌ 403 |
| SDK Config | ✅ | ✅ | ❌ 403 |

### Billing (Permission Override Available)

| Feature | `admin` | `publisher` | `readonly` | With `billing:view` |
|---------|---------|-------------|------------|---------------------|
| Usage | ✅ | ✅ | ❌ | ✅ |
| Invoices | ✅ | ✅ | ❌ | ✅ |
| Settings | ✅ | ✅ | ❌ | ✅ |

## Implementation

### Core RBAC Library (`console/src/lib/rbac.ts`)

```typescript
export type Role = 'admin' | 'publisher' | 'readonly'

export function isCustomerRole(value: unknown): value is Role {
  return value === 'admin' || value === 'publisher' || value === 'readonly'
}

export function hasRole(userRole: Role | null | undefined, allowed: Role[]): boolean {
  if (!userRole) return false
  return allowed.includes(userRole)
}

export function assertRole(userRole: Role | null | undefined, allowed: Role[], message?: string) {
  if (!hasRole(userRole, allowed)) {
    throw new RbacError(message || 'Forbidden', `requires-role:${allowed.join('|')}`)
  }
}
```

### Settings Layout Protection (`console/src/app/settings/layout.tsx`)

Server-side RBAC enforcement for ALL settings routes:

```typescript
const ALLOWED_ROLES: Role[] = ['admin', 'publisher']

export default async function SettingsLayout({ children }) {
  const session = await getServerSession(authOptions)
  
  if (!session?.user) {
    redirect('/login?next=/settings')
  }

  const role = session.user.role ?? null
  
  if (!hasRole(role, ALLOWED_ROLES)) {
    redirect('/403')  // readonly users blocked
  }

  return <>{children}</>
}
```

### Billing Access Control (`console/src/app/billing/access.ts`)

Supports permission-based override for readonly users:

```typescript
export const BILLING_ALLOWED_ROLES: Role[] = ['admin', 'publisher']
export const BILLING_ALLOWED_PERMISSIONS = ['billing:view', 'billing:manage']

export function canAccessBilling(user: SessionUser | null): boolean {
  if (!user) return false
  
  const role = (user.role ?? 'readonly') as Role
  const hasRoleAccess = BILLING_ALLOWED_ROLES.includes(role)
  const hasPermission = (user.permissions ?? []).some(p =>
    BILLING_ALLOWED_PERMISSIONS.includes(p)
  )
  
  return hasRoleAccess || hasPermission
}
```

## Operator Access (Separate System)

**CRITICAL:** Customer roles do NOT grant operator access.

| System | Authentication | Access |
|--------|----------------|--------|
| Customer Console | NextAuth session with role | Organization features |
| Operator Control Plane | Separate operator auth | System-wide admin |

The operator control plane is hosted on a separate subdomain (`admin.apexmediation.ee`) and uses `useAdminGate.ts` for authentication:

```typescript
// console/src/lib/useAdminGate.ts
// Completely separate from customer RBAC
```

## Security Measures

### 1. Server-Side Enforcement

All protected routes use server-side checks via layouts:
- `settings/layout.tsx` — RBAC for settings
- `billing/layout.tsx` — RBAC for billing (with permission override)
- `admin/layout.tsx` — Redirects to operator control plane

### 2. Direct URL Protection

Layouts prevent direct URL access bypass:
```
User navigates to /settings/security directly
    ↓
settings/layout.tsx runs server-side
    ↓
Checks session and role
    ↓
readonly → redirect to /403
admin/publisher → render page
```

### 3. Case-Sensitive Validation

Role strings are validated case-sensitively to prevent typosquatting:
```typescript
isCustomerRole('Admin')  // false
isCustomerRole('ADMIN')  // false
isCustomerRole('admin')  // true
```

### 4. Type-Safe Injection Prevention

The RBAC system prevents object/array injection:
```typescript
hasRole(['admin'] as unknown as Role, ['admin'])  // false
hasRole({ toString: () => 'admin' } as unknown as Role, ['admin'])  // false
```

## Testing

### Run RBAC Tests
```bash
cd console
npm test -- --testPathPattern="rbac"
```

### Test Coverage

28 edge case tests covering:
- Valid/invalid role validation
- Null/undefined handling
- Role hierarchy enforcement
- Permission-based access (billing)
- Security injection attempts
- Case sensitivity

## Adding New Protected Routes

### 1. Using Layout (Recommended)

Create a `layout.tsx` in the route directory:

```typescript
// app/new-feature/layout.tsx
import { getServerSession } from 'next-auth'
import { redirect } from 'next/navigation'
import { hasRole, type Role } from '@/lib/rbac'
import { authOptions } from '@/lib/auth/options'

const ALLOWED_ROLES: Role[] = ['admin', 'publisher']

export default async function NewFeatureLayout({ children }) {
  const session = await getServerSession(authOptions)
  
  if (!session?.user) {
    redirect('/login?next=/new-feature')
  }
  
  if (!hasRole(session.user.role, ALLOWED_ROLES)) {
    redirect('/403')
  }
  
  return <>{children}</>
}
```

### 2. Client-Side Check (For UI Elements)

```typescript
'use client'
import { useSession } from 'next-auth/react'
import { hasRole } from '@/lib/rbac'

export default function FeaturePage() {
  const { data: session } = useSession()
  const canEdit = hasRole(session?.user?.role, ['admin', 'publisher'])
  
  return (
    <div>
      {canEdit && <button>Edit</button>}
    </div>
  )
}
```

## Troubleshooting

### User sees 403 unexpectedly
1. Check user's role in database
2. Verify session is active
3. Check if route layout has correct ALLOWED_ROLES

### Role not updating after change
1. User must log out and log back in
2. Session stores role at login time
3. Force session refresh if needed

### Permission override not working
1. Check user has explicit permission in `permissions` array
2. Verify billing access function is used correctly
3. Check feature flag is enabled
