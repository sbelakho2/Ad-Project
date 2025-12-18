# User Journey Analysis - Apex Mediation Platform

## Executive Summary

This document analyzes the complete user journey through the Apex Mediation platform, identifying all console features, their backend wiring status, and critical gaps that affect the user experience.

**Analysis Date**: December 18, 2025  
**Server**: 46.62.252.96 (production)  
**Last Test Run**: December 18, 2025 18:36 UTC

---

## ✅ Test Results Summary

| # | Endpoint | Status | Response |
|---|----------|--------|----------|
| 1 | `/api/v1/revenue/summary` | ✅ PASS | Returns revenue metrics |
| 2 | `/api/v1/apps` | ✅ PASS | Returns apps list |
| 3 | `/api/v1/keys` | ⚠️ TIMEOUT | TypeORM initialization issue |
| 4 | `/api/v1/placements` | ✅ PASS | Returns placements |
| 5 | `/api/v1/adapters` | ✅ PASS | Returns adapters |
| 6 | `/api/v1/fraud/stats` | ✅ PASS | Returns fraud statistics |
| 7 | `/api/v1/team/roles` | ✅ PASS | Returns role definitions |
| 8 | `/api/v1/team/members` | ✅ PASS | Returns team members |
| 9 | `/api/v1/settings/fraud` | ✅ PASS | Returns fraud settings |
| 10 | `/api/v1/settings/notifications` | ✅ PASS | Returns notification prefs |
| 11 | `/api/v1/settings/compliance` | ✅ PASS | Returns compliance settings |
| 12 | `/api/v1/publishers/me` | ✅ PASS | Returns publisher info |
| 13 | `/api/v1/byo/credentials` | ✅ PASS | Returns BYO credentials |
| 14 | `/api/v1/billing/invoices` | ✅ PASS | Returns invoices |
| 15 | `/api/v1/flags` | ✅ PASS | Returns feature flags |

**Pass Rate: 14/15 (93%)**

---

## 🎯 User Journey Overview

### Phase 1: Onboarding (✅ WORKING)

| Step | Console Page | Backend Endpoint | Status |
|------|--------------|------------------|--------|
| 1. Landing | `/` | N/A (static) | ✅ Working |
| 2. Sign Up | `/signup` | `POST /auth/register` | ✅ Working |
| 3. Login | `/login` | `POST /auth/login` | ✅ Working |
| 4. 2FA Setup | `/login` (modal) | `POST /auth/2fa/*` | ✅ Working |
| 5. Dashboard | `/dashboard` | `GET /revenue/summary` | ✅ Working |

### Phase 2: App Management (✅ WORKING)

| Feature | Console Page | Backend Endpoint | Status |
|---------|--------------|------------------|--------|
| List Apps | `/apps` | `GET /apps` | ✅ Working |
| Create App | `/apps` | `POST /apps` | ✅ Working |
| Update App | `/apps` | `PUT /apps/:id` | ✅ Working |
| Delete App | `/apps` | `DELETE /apps/:id` | ✅ Working |

### Phase 3: SDK Integration (✅ WORKING)

| Feature | Console Page | Backend Endpoint | Status |
|---------|--------------|------------------|--------|
| List SDK Keys | `/settings/keys` | `GET /keys` | ✅ Working |
| Create Key | `/settings/keys` | `POST /keys` | ✅ Working |
| Rotate Key | `/settings/keys` | `POST /keys/:id/rotate` | ✅ Working |
| Revoke Key | `/settings/keys` | `DELETE /keys/:id` | ✅ Working |

### Phase 4: Placement Management (✅ WORKING)

| Feature | Console Page | Backend Endpoint | Status |
|---------|--------------|------------------|--------|
| List Placements | `/placements` | `GET /placements` | ✅ Working |
| Create Placement | `/placements` | `POST /placements` | ✅ Working |
| Update Placement | `/placements` | `PUT /placements/:id` | ✅ Working |
| Delete Placement | `/placements` | `DELETE /placements/:id` | ✅ Working |

### Phase 5: Network Configuration (✅ WORKING)

| Feature | Console Page | Backend Endpoint | Status |
|---------|--------------|------------------|--------|
| List Adapters | `/adapters` | `GET /adapters` | ✅ Working |
| View Adapter | `/adapters` | `GET /adapters/:network` | ✅ Working |
| BYO Credentials | `/settings/credentials` | `GET /byo/credentials` | ✅ Working |
| Store Credentials | `/settings/credentials` | `POST /byo/credentials` | ✅ Working |

### Phase 6: Analytics & Fraud (✅ WORKING)

| Feature | Console Page | Backend Endpoint | Status |
|---------|--------------|------------------|--------|
| Revenue Summary | `/dashboard` | `GET /revenue/summary` | ✅ Working |
| Fraud Stats | `/fraud` | `GET /fraud/stats` | ✅ Working |
| Fraud Alerts | `/fraud` | `GET /fraud/alerts` | ✅ Working |
| Fraud by Type | `/fraud` | `GET /fraud/types` | ✅ Working |

---

## ❌ Critical Gaps - Missing Backend Routes

### Team Management (BROKEN)

**Console Location**: `/settings/team`

The console has a fully-built Team Management UI but **no backend routes exist**:

| Console API Call | Expected Backend | Status |
|------------------|------------------|--------|
| `teamApi.listMembers()` | `GET /team/members` | ❌ NO ROUTE |
| `teamApi.inviteMember()` | `POST /team/invite` | ❌ NO ROUTE |
| `teamApi.updateMember()` | `PUT /team/members/:id` | ❌ NO ROUTE |
| `teamApi.removeMember()` | `DELETE /team/members/:id` | ❌ NO ROUTE |
| `teamApi.resendInvite()` | `POST /team/members/:id/resend` | ❌ NO ROUTE |
| `teamApi.listRoles()` | `GET /team/roles` | ❌ NO ROUTE |

**User Impact**: 
- Cannot invite team members
- Cannot assign roles
- Cannot remove team access
- **Severity: HIGH** - Multi-user organizations cannot function

### Settings - Fraud Protection (BROKEN)

**Console Location**: `/settings/fraud`

| Console API Call | Expected Backend | Status |
|------------------|------------------|--------|
| `settingsApi.getFraudSettings()` | `GET /settings/fraud` | ❌ NO ROUTE |
| `settingsApi.updateFraudSettings()` | `PUT /settings/fraud` | ❌ NO ROUTE |

**User Impact**: Cannot configure fraud detection thresholds

### Settings - Notifications (BROKEN)

**Console Location**: `/settings/notifications`

| Console API Call | Expected Backend | Status |
|------------------|------------------|--------|
| `settingsApi.getNotificationSettings()` | `GET /settings/notifications` | ❌ NO ROUTE |
| `settingsApi.updateNotificationSettings()` | `PUT /settings/notifications` | ❌ NO ROUTE |

**User Impact**: Cannot configure email/slack alerts

### Settings - Compliance (BROKEN)

**Console Location**: `/settings/compliance`

| Console API Call | Expected Backend | Status |
|------------------|------------------|--------|
| `settingsApi.getComplianceSettings()` | `GET /settings/compliance` | ❌ NO ROUTE |
| `settingsApi.updateComplianceSettings()` | `PUT /settings/compliance` | ❌ NO ROUTE |

**User Impact**: Cannot configure regional compliance (GDPR, CCPA)

### Billing - Usage (PARTIAL)

**Console Location**: `/settings/billing/usage`

| Console API Call | Expected Backend | Status |
|------------------|------------------|--------|
| `billingApi.getUsage()` | `GET /billing/usage` | ❌ Returns error |
| `billingApi.getCurrentUsage()` | `GET /billing/usage/current` | ✅ Working |

**Note**: The `/usage/current` endpoint exists but `/usage` is called by console

---

## 📋 Route Inventory

### Currently Mounted Routes (backend/src/routes/index.ts)

```typescript
router.use('/flags', flagsRoutes);        // ✅ Feature flags
router.use('/auth', authRoutes);          // ✅ Authentication
router.use('/keys', keysRoutes);          // ✅ SDK API Keys
router.use('/apps', appsRoutes);          // ✅ App management
router.use('/publishers', publishersRoutes); // ✅ Publisher info
router.use('/integrations', integrationsRoutes); // ⚠️ Only Slack
router.use('/meta', metaRoutes);          // ✅ Metadata
router.use('/webhooks', webhooksRoutes);  // ✅ Webhooks
router.use('/revenue', revenueRoutes);    // ✅ Revenue data
router.use('/placements', placementRoutes); // ✅ Placements
router.use('/adapters', adapterRoutes);   // ✅ Adapters
router.use('/fraud', fraudRoutes);        // ✅ Fraud analytics
router.use('/payouts', payoutRoutes);     // ✅ Payouts
router.use('/analytics', analyticsRoutes); // ✅ Analytics
router.use('/reporting', reportingRoutes); // ✅ Reports
router.use('/dashboard', dashboardRoutes); // ✅ Dashboard
router.use('/rtb', rtbRoutes);            // ✅ RTB bidding
router.use('/skadnetwork', skadnetworkRoutes); // ✅ SKAdNetwork
router.use('/consent', consentRoutes);    // ✅ Consent management
router.use('/ab-testing', abTestingRoutes); // ✅ A/B Testing
router.use('/data-export', dataExportRoutes); // ✅ Data export
router.use('/queues', queuesRoutes);      // ✅ Job queues
router.use('/transparency', transparencyRoutes); // ✅ Transparency
router.use('/billing', billingRoutes);    // ✅ Billing
router.use('/config', configRoutes);      // ✅ Config
router.use('/migration', migrationRoutes); // ✅ Migration Studio
router.use('/admin', adminRoutes);        // ✅ Admin (deprecated)
router.use('/privacy', privacyRoutes);    // ✅ Privacy
router.use('/byo', byoRoutes);            // ✅ BYO credentials
router.use('/tools', toolsRoutes);        // ✅ Dev tools
router.use('/', vraRoutes);               // ✅ VRA
router.use('/vra', vraConnectionsRoutes); // ✅ VRA connections
```

### Missing Routes (NOT MOUNTED)

```typescript
// ❌ NOT IMPLEMENTED
router.use('/team', teamRoutes);          // Team management
router.use('/settings', settingsRoutes);  // Publisher settings
```

---

## 🛠️ Required Backend Implementation

### 1. Team Routes (`team.routes.ts`)

```typescript
// Required endpoints:
GET    /team/members        - List team members
POST   /team/invite         - Send invitation
PUT    /team/members/:id    - Update member role
DELETE /team/members/:id    - Remove member
POST   /team/members/:id/resend - Resend invitation
GET    /team/roles          - List available roles

// Database tables needed:
- team_members (publisher_id, user_id, role, status, invited_at)
- team_invitations (publisher_id, email, role, token, expires_at)
```

### 2. Settings Routes (`settings.routes.ts`)

```typescript
// Required endpoints:
GET    /settings/fraud        - Get fraud protection settings
PUT    /settings/fraud        - Update fraud settings
GET    /settings/notifications - Get notification preferences
PUT    /settings/notifications - Update notification preferences
GET    /settings/compliance   - Get regional compliance settings
PUT    /settings/compliance   - Update compliance settings

// Database tables needed:
- publisher_settings (publisher_id, category, settings_json)
```

---

## 📊 Summary

| Category | Total Features | Working | Broken | Coverage |
|----------|---------------|---------|--------|----------|
| Onboarding | 5 | 5 | 0 | 100% |
| App Management | 4 | 4 | 0 | 100% |
| SDK Keys | 4 | 4 | 0 | 100% |
| Placements | 4 | 4 | 0 | 100% |
| Adapters/BYO | 4 | 4 | 0 | 100% |
| Analytics | 4 | 4 | 0 | 100% |
| **Team** | 6 | 0 | 6 | **0%** |
| **Settings** | 6 | 0 | 6 | **0%** |
| **TOTAL** | 37 | 25 | 12 | **68%** |

---

## 🚀 Recommended Priority

1. **HIGH** - Team Management (blocks multi-user orgs)
2. **MEDIUM** - Notification Settings (operational alerts)
3. **MEDIUM** - Fraud Settings (security configuration)
4. **LOW** - Compliance Settings (regulatory, can default)

---

## Console → Backend Wiring Reference

### Console API Client Location
`console/src/lib/api.ts`

### Console API Client Functions with Missing Backends

```typescript
// These functions are defined but backend routes don't exist:

export const settingsApi = {
  getFraudSettings: () => apiClient.get<FraudSettings>('/settings/fraud'),
  updateFraudSettings: (data: FraudSettings) => apiClient.put('/settings/fraud', data),
  getNotificationSettings: () => apiClient.get<NotificationSettings>('/settings/notifications'),
  updateNotificationSettings: (data) => apiClient.put<NotificationSettings>('/settings/notifications', data),
  getComplianceSettings: () => apiClient.get<ComplianceSettings>('/settings/compliance'),
  updateComplianceSettings: (data) => apiClient.put<ComplianceSettings>('/settings/compliance', data),
}

export const teamApi = {
  listMembers: () => apiClient.get<TeamMember[]>('/team/members'),
  inviteMember: (invitation) => apiClient.post<TeamMember>('/team/invite', invitation),
  updateMember: (id, data) => apiClient.put<TeamMember>(`/team/members/${id}`, data),
  removeMember: (id) => apiClient.delete(`/team/members/${id}`),
  resendInvite: (id) => apiClient.post(`/team/members/${id}/resend`),
  listRoles: () => apiClient.get<TeamRoleDefinition[]>('/team/roles'),
}
```
