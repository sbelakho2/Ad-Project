# Control Plane Architecture

## Overview

The Control Plane is a **completely separate service** from the customer-facing backend API. It runs on its own hostname (`admin.apexmediation.ee`), uses its own authentication system, and has no shared code with the customer API.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         NETWORK ARCHITECTURE                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌──────────────────┐         ┌──────────────────────┐                 │
│  │    CUSTOMERS     │         │     OPERATORS        │                 │
│  │   (any IP)       │         │   (allowlisted IPs)  │                 │
│  └────────┬─────────┘         └──────────┬───────────┘                 │
│           │                              │                              │
│           ▼                              ▼                              │
│  ┌──────────────────┐         ┌──────────────────────┐                 │
│  │ console.apex...  │         │ admin.apexmediation.ee│                │
│  │ api.apex...      │         │ (IP allowlisted)     │                 │
│  └────────┬─────────┘         └──────────┬───────────┘                 │
│           │                              │                              │
│           ▼                              ▼                              │
│  ┌──────────────────┐         ┌──────────────────────┐                 │
│  │ CUSTOMER BACKEND │         │    CONTROL PLANE     │                 │
│  │     Port 8080    │         │      Port 4000       │                 │
│  │                  │         │                      │                 │
│  │  - Customer JWT  │         │  - Operator JWT      │                 │
│  │  - No /admin     │         │  - IP allowlist      │                 │
│  │  - Customer data │         │  - Full telemetry    │                 │
│  └────────┬─────────┘         └──────────┬───────────┘                 │
│           │                              │                              │
│           ▼                              ▼                              │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                         POSTGRESQL                              │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │   │
│  │  │ apex_app    │  │ apex_readonly│  │ apex_operator/breakglass│  │   │
│  │  │ (customer)  │  │ (monitoring) │  │ (admin actions)         │  │   │
│  │  └─────────────┘  └─────────────┘  └─────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

## Security Layers

### 1. Network Isolation

- **Separate hostname**: `admin.apexmediation.ee` vs `api.apexmediation.ee`
- **Separate TLS certificate**: Independent certificate lifecycle
- **IP Allowlisting**: Control plane only accessible from whitelisted IPs
  - Caddy performs first-layer IP check
  - Application performs second-layer IP check
  - Returns HTTP 404 (not 403) for denied IPs (stealth mode)

### 2. Authentication Separation

| Aspect | Customer API | Control Plane |
|--------|--------------|---------------|
| JWT Secret | `JWT_SECRET` | `OPERATOR_JWT_SECRET` |
| Token Expiry | 7 days | 8 hours |
| User Store | `users` table | Environment variables |
| Password Storage | bcrypt in DB | bcrypt in env |
| Token Format | Customer claims | Operator claims |

**Critical**: Customer JWTs are NOT accepted by the control plane. There is no token exchange or shared secret.

### 3. Database Role Separation

The control plane uses three separate database roles with different privileges:

```sql
-- Read-only (monitoring, telemetry)
CREATE ROLE apex_readonly WITH LOGIN PASSWORD '...';
GRANT CONNECT ON DATABASE apexmediation TO apex_readonly;
GRANT USAGE ON SCHEMA public TO apex_readonly;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO apex_readonly;
-- Specifically grant access to pg_stat views
GRANT pg_read_all_stats TO apex_readonly;

-- Operator (partition management, safe writes)
CREATE ROLE apex_operator WITH LOGIN PASSWORD '...';
GRANT CONNECT ON DATABASE apexmediation TO apex_operator;
GRANT USAGE ON SCHEMA public TO apex_operator;
GRANT SELECT, INSERT, UPDATE ON system_state, audit_logs TO apex_operator;
GRANT CREATE ON SCHEMA public TO apex_operator; -- For partition creation

-- Break-glass (emergency session termination)
CREATE ROLE apex_breakglass WITH LOGIN PASSWORD '...';
GRANT CONNECT ON DATABASE apexmediation TO apex_breakglass;
GRANT pg_signal_backend TO apex_breakglass;
```

## Action Tiers

All control plane actions follow a tiered permission model with typed confirmations for dangerous operations.

### Tier 0 - Always Safe
No confirmation required. Read-only or log-only operations.

| Action | Description |
|--------|-------------|
| `capture_incident_snapshot` | Capture system state for debugging |
| `run_deep_healthcheck` | Comprehensive health check |
| `view_telemetry` | View DB/host/app metrics |

### Tier 1 - Operator Gated
Requires operator authentication. Reversible operations.

| Action | Description |
|--------|-------------|
| `enter_degraded_mode` | Set system to degraded mode |
| `exit_degraded_mode` | Return system to normal mode |
| `create_future_partitions` | Create partition tables ahead |
| `throttle_background_jobs` | Reduce job concurrency |

### Tier 2 - Admin + Typed Confirmation
Requires operator authentication AND typed confirmation string.

| Action | Confirmation Required |
|--------|----------------------|
| `terminate_session` | `TERMINATE SESSION {pid}` |
| `drop_old_partition` | `DROP PARTITION {name}` |
| `restart_service` | `RESTART SERVICE {name}` |

Example:
```json
POST /api/v1/actions/terminate-session
{
  "pid": 12345,
  "confirmation": "TERMINATE SESSION 12345"
}
```

## Audit Logging

All control plane actions are logged with 7-year retention (Estonian financial compliance).

```typescript
interface AuditEvent {
  actor: string;       // Operator username
  action: string;      // Action name
  target?: string;     // Affected resource
  preState: unknown;   // State before action
  postState: unknown;  // State after action
  result: 'success' | 'failure' | 'denied';
  reason?: string;     // Failure reason
  ip: string;          // Operator IP
  userAgent?: string;  // Browser/client info
  timestamp: Date;
}
```

## API Reference

### Authentication

```
POST /api/v1/auth/login
Body: { "username": "...", "password": "..." }
Response: { "token": "...", "expiresIn": "8h" }
```

### Telemetry

```
GET /api/v1/telemetry/database    # DB metrics, sessions, queries
GET /api/v1/telemetry/host        # Disk, memory, CPU
GET /api/v1/telemetry/app         # API latency, errors, pools
GET /api/v1/dashboard             # Comprehensive overview
```

### Partition Management

```
GET /api/v1/partitions                      # All partition status
GET /api/v1/partitions/:table               # Single table status
POST /api/v1/partitions/:table/create       # Create future partitions
POST /api/v1/partitions/auto-maintain       # Auto-maintain all
```

### Actions

```
POST /api/v1/actions/snapshot               # Tier 0: Capture snapshot
POST /api/v1/actions/healthcheck            # Tier 0: Deep healthcheck
POST /api/v1/actions/degraded-mode/enter    # Tier 1: Enter degraded mode
POST /api/v1/actions/degraded-mode/exit     # Tier 1: Exit degraded mode
POST /api/v1/actions/terminate-session      # Tier 2: Kill DB session
POST /api/v1/actions/drop-partition         # Tier 2: Drop partition
```

## Deployment

### Prerequisites

1. Create database roles:
```sql
-- Run as postgres superuser
CREATE ROLE apex_readonly WITH LOGIN PASSWORD 'readonly_password';
CREATE ROLE apex_operator WITH LOGIN PASSWORD 'operator_password';
CREATE ROLE apex_breakglass WITH LOGIN PASSWORD 'breakglass_password';
-- Grant appropriate permissions (see Database Role Separation above)
```

2. Generate operator credentials:
```bash
# Generate bcrypt hash for each operator
node -e "console.log(require('bcryptjs').hashSync('secure_password', 12))"
```

3. Set environment variables:
```bash
export OPERATOR_JWT_SECRET="$(openssl rand -base64 32)"
export OPERATOR_CREDENTIALS='{"admin":"$2a$12$hash...","backup":"$2a$12$hash..."}'
export CONTROL_PLANE_ALLOWED_IPS="1.2.3.4/32,5.6.7.8/32"
```

### Docker Deployment

```bash
# Build and run control plane
docker-compose -f docker-compose.yml -f docker-compose.control-plane.yml up -d control-plane
```

### Caddy Configuration

```caddyfile
admin.apexmediation.ee {
  # IP allowlist (first-layer check)
  @allowed {
    remote_ip 1.2.3.4/32 5.6.7.8/32
  }
  
  handle @allowed {
    reverse_proxy control-plane:4000
  }
  
  # Return 404 for non-allowed IPs (stealth mode)
  respond 404
}
```

## Migration Notes

The `/admin` routes on the customer backend have been **completely removed**. They now return HTTP 410 GONE with a message directing operators to the control plane.

```
GET /api/v1/admin/*

Response: 410 Gone
{
  "error": "GONE",
  "message": "Operator functionality moved to control plane service"
}
```

This ensures:
- ✅ No operator code on customer API
- ✅ No shared authentication realm
- ✅ No risk of privilege escalation
- ✅ Complete architectural separation
