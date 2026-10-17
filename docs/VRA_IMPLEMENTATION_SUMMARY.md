# VRA Production Implementation Summary

## Overview

The VRA (Vendor Revenue Assurance) system has been fully implemented to enable publishers to automatically reconcile their ad revenue across networks. This document summarizes the implementation.

## Problem Solved

Previously, VRA had:
- **Fake coverage metrics** - Binary 0% or 100%, not based on actual data
- **Zero variance always** - Expected == Paid by design (mirrored data)
- **No network integrations** - No way to pull actual network statements
- **No automation** - Manual-only reconciliation
- **No publisher onboarding** - No way for publishers to connect networks

## Implementation

### 1. Database Schema (`backend/migrations/20251212_vra_network_connections.sql`)

New tables created:
- `vra_network_connections` - Stores encrypted network credentials
- `vra_sync_history` - Audit trail of network syncs
- `vra_reconciliation_runs` - Records of reconciliation executions
- `vra_alerts` - Variance alerts for publishers

### 2. Network Connector Service (`backend/src/services/vra/networkConnectorService.ts`)

Manages connections to 6 supported ad networks:
- **AdMob** (OAuth 2.0)
- **Unity Ads** (API Key)
- **AppLovin** (API Key)
- **IronSource** (API Key)
- **Meta Audience Network** (OAuth 2.0)
- **Vungle** (API Key)

Features:
- AES-256-CBC encrypted credential storage
- OAuth flow support
- Connection validation
- Automatic token refresh

### 3. Sync Service (`backend/src/services/vra/vraSyncService.ts`)

Orchestrates data retrieval and reconciliation:
- Pulls revenue statements from each network
- Normalizes data to USD
- Stores in `recon_statements_norm` table
- Runs reconciliation against SDK events
- Generates variance alerts

### 4. Scheduler (`backend/src/services/vra/vraScheduler.ts`)

Automated jobs:
- **Sync job**: Every 6 hours, syncs all active connections
- **Reconciliation job**: Daily at 8am UTC, runs for all publishers

### 5. API Routes (`backend/src/routes/vraConnections.routes.ts`)

New endpoints:
```
GET  /api/v1/vra/networks     - List supported networks
GET  /api/v1/vra/connections  - List publisher's connections
POST /api/v1/vra/connections  - Create new connection
DEL  /api/v1/vra/connections/:id - Remove connection
POST /api/v1/vra/connections/:id/sync - Manual sync trigger
POST /api/v1/vra/connections/:id/validate - Test connection
GET  /api/v1/vra/oauth/:network - Start OAuth flow
GET  /api/v1/vra/oauth/:network/callback - OAuth callback
POST /api/v1/vra/reconcile   - Manual reconciliation
GET  /api/v1/vra/alerts      - Get alerts
```

### 6. Fixed VRA Service (`backend/src/services/vra/vraService.ts`)

`getOverview()` now calculates real metrics:
- **Coverage**: `networks_with_statements / networks_with_events * 100`
- **Expected**: Sum of revenue from SDK events (`revenue_events` table)
- **Paid**: Sum of revenue from network statements (`recon_statements_norm` table)
- **Variance**: `(expected - paid) / expected * 100`

### 7. Console UI

**Network Settings Page** (`console/src/app/settings/networks/page.tsx`):
- Lists all supported networks with status
- OAuth flow initiation for AdMob/Meta
- API key credential forms for other networks
- Connection status and sync triggers

**Recon Dashboard** (`console/src/app/recon/page.tsx`):
- Onboarding wizard for new publishers
- Key metrics: coverage, expected, paid, variance
- Per-network variance breakdown
- Active alerts display
- Manual reconciliation trigger

## Environment Variables Required

```bash
# Encryption key for credentials (32 bytes)
VRA_ENCRYPTION_KEY=your-32-byte-encryption-key

# Enable scheduler (default: false in dev)
VRA_SCHEDULER_ENABLED=true

# OAuth credentials for networks
ADMOB_CLIENT_ID=...
ADMOB_CLIENT_SECRET=...
META_CLIENT_ID=...
META_CLIENT_SECRET=...
```

## How Publishers Use It

1. **Connect Networks**: Go to Settings > Networks
2. **Add Credentials**: OAuth or API keys
3. **Wait for Sync**: Automatic every 6 hours
4. **View Dashboard**: Recon page shows variances
5. **Get Alerts**: Notified of underpayments >5%

## Variance Detection

The system detects:
- **Underpay**: Network paid less than SDK expected
- **Overpay**: Network paid more than SDK expected
- **Missing Network**: SDK events but no network statement
- **Missing SDK**: Network statement but no SDK events

Alerts are created when:
- Single network variance >5%
- Total variance >$1000 USD

## Files Created/Modified

**New Files:**
- `backend/migrations/20251212_vra_network_connections.sql`
- `backend/src/services/vra/networkConnectorService.ts`
- `backend/src/services/vra/vraSyncService.ts`
- `backend/src/services/vra/vraScheduler.ts`
- `backend/src/routes/vraConnections.routes.ts`
- `console/src/app/settings/networks/page.tsx`
- `console/src/app/recon/page.tsx`

**Modified Files:**
- `backend/src/services/vra/vraService.ts` - Real coverage/variance
- `backend/src/services/vra/vraTypes.ts` - New types
- `backend/src/routes/index.ts` - Added VRA routes

## Deployment Status

**Infrastructure:** Hetzner (migrated from DigitalOcean December 2025)

| Component | Server | Status |
|-----------|--------|--------|
| VRA Tables | DB Server (203.0.113.12) | ✅ Deployed |
| VRA Service | App Server (203.0.113.10) | ✅ Running |
| VRA Scheduler | App Server | ✅ Active |
| Console UI | App Server | ✅ Deployed |

- ✅ Migration run on production (Hetzner DB Server)
- ✅ Backend restarted and healthy
- ✅ Tables created: vra_network_connections, vra_sync_history, vra_reconciliation_runs, vra_alerts
- ✅ Console UI deployed with network settings page
- ✅ Scheduler running (6-hour sync cycle)

## Configuration

```bash
# Required environment variables (App Server /opt/adproject/backend/.env)
VRA_ENCRYPTION_KEY=<32-byte-key>
VRA_SCHEDULER_ENABLED=true

# Optional: OAuth credentials for supported networks
ADMOB_CLIENT_ID=...
ADMOB_CLIENT_SECRET=...
META_CLIENT_ID=...
META_CLIENT_SECRET=...
```
