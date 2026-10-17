# Control Plane Operations Guide

**Last Updated:** January 25, 2026

## Overview
The ApexMediation Control Plane (`admin.apexmediation.ee`) is the central nerve center for platform operators. It is architected as a separate service with elevated privileges, strictly decoupled from the customer-facing API to ensure maximum security and reliability.

## 🏗️ Sales Automation Architecture

> **⚠️ CRITICAL: The Control Plane on App Server is modularized and uses ApexTunnel for cross-server communication.**

| Component | Server | Port | Role |
|-----------|--------|------|------|
| Control Plane | App (203.0.113.10) | 4000 | Refactored modular controllers |
| **Sales API** | **Test (203.0.113.11)** | 4001 | **ALL orchestration, spintax, Thompson sampler, scoring** |
| Edge Worker | Test (203.0.113.11) | 3500 | ML inference (ONNX 9-Dim, classification) |
| DB Access | Test→DB | 5432 | Direct P2P connectivity via ApexTunnel |

### ApexTunnel (P2P Mesh Networking)

| Service | Server | Purpose |
|---------|--------|---------|
| `apextunnel.service` | All Nodes | P2P encrypted mesh replacing legacy SSH tunnels |
| Identity | Ed25519 | Mutual auth using `APEX_CONFIG_SIGNING_KEY` |
| Protocol | Noise (UDP) | Low-latency, high-availability P2P connectivity |

### Checking Sales Automation Health

```bash
# From App Server (via ApexTunnel)
curl -s http://localhost:4000/api/autonomous/health

# Direct to Test Server (internal P2P IP)
curl -s http://10.0.0.3:4001/api/autonomous/health

# Check ApexTunnel status
./scripts/ops/hetzner_ssh.sh app -- "systemctl status apextunnel"

# Check Sales API on Test Server
./scripts/ops/hetzner_ssh.sh test -- "pm2 status sales-api"
```

## 🔐 Access & Security
- **Authentication**: Uses a separate JWT realm for operators. Credentials are not shared with the customer database.
- **Device Approval**: New administrative devices require email-based verification.
- **Audit Logging**: Every action (viewing sensitive data, generating reports, infrastructure changes) is logged to the `audit_logs` table with a mandatory 7-year retention policy.
- **Tiered Database Access**:
  - **Read-Only**: Dashboard and telemetry views.
  - **Operator**: Standard management tasks (partition creation, job control).
  - **Break-Glass**: Emergency remediation (killing sessions, forced drops). Requires explicit justification.

## ⚖️ Compliance & Accounting

### 1. VAT Reporting (KMD)
Located under the **Compliance** tab.
- **Quarterly Reports**: Automatically aggregates sales and VAT from the `invoices` table.
- **KMD INF**: Specifically tracks and lists transactions exceeding 1000 EUR as required by Estonian law.
- **PDF Generation**: Branded PDF reports can be downloaded for manual submission to the e-MTA portal.

### 2. Annual Financial Reports
- **Ledger Integration**: Financials (Assets, Liabilities, Equity, Profit/Loss) are derived in real-time from the `account_balances` ledger view.
- **Archival**: Generated reports are archived in the `financial_documents` table with a SHA256 hash for integrity verification.

### 3. TSD (Payroll & Tax)
- **Payroll Tracking**: Aggregates `payout` transactions from the ledger to estimate social and income tax liabilities.

## 📊 Business & CEO Analytics

### Core Performance
- **Real-Time Volume**: Monitor Impressions and Revenue (USD) with WoW (Week-over-Week) and DoD (Day-over-Day) delta tracking.
- **Active footprint**: Real-time count of active publishers and VRA experiments.
- **Fraud Drift**: Dynamic PSI monitoring for 9-dimensional ML feature shapes. Alerts on distribution shifts vs. 7-day baseline.

### CEO Metrics
- **Runway**: Estimated months of operation remaining based on current bank balances and 90-day average burn rate.
- **Cash Flow**: 30-day monitoring of Inflows (paid invoices) vs Outflows (expenses).
- **LTV & Churn**: Automated calculation of Average Customer Lifetime Value and 30-day publisher churn rate.

### Advanced Analytics
- **Cohort Analysis**: Tracks publisher retention by signup month over a 6-month period.
- **Revenue Forecasting**: Linear growth modeling providing a 6-month outlook with upper/lower confidence bounds.

## 🏦 Bank Reconciliation
The **Reconciliation** view allows operators to match external bank statements (SEPA/Wise) against internal records.
- **Automated Matching**: The system correlates bank references and amounts with `invoices` and `expenses`.
- **Operator Review**: Matches are presented with confidence scores. A single click confirms the match and updates the ledger status.

## 🏗️ Infrastructure Management

### 1. Partition Management
High-volume tables (`ad_events`, `bid_logs`) use PostgreSQL native partitioning.
- **Health Checks**: The dashboard warns if future partitions are missing.
- **Auto-Maintenance**: One-click creation of partitions for the next 3 months.
- **Retention**: Old partitions are identified for dropping based on the `DATA_RETENTION_POLICIES.md`.

### 2. Job Orchestration
- **Autopilot**: Monitor the status of background cron jobs (analytics aggregation, cache warming, etc.).
- **Migration Studio**: Track the throughput and error rates of live data migrations.

### 3. System Telemetry
- **Database**: Monitor top queries, active sessions, and WAL/Replication status.
- **App**: P99 latency and error rates fetched from the backend Prometheus endpoint.
- **Host**: Disk, CPU, and RAM utilization of the underlying VM.

## 🛠️ Support Tools
- **Impersonation**: Operators can launch a short-lived, read-only session as any publisher to troubleshoot issues. All impersonation sessions are audited and require a reason.
