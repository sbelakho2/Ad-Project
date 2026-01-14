# ApexMediation Control Plane

Internal management service for the ApexMediation platform. Handles administrative tasks, compliance reporting, and infrastructure orchestration.

## 🏗️ Architecture

The Control Plane is a separate Node.js service with elevated privileges to perform sensitive operations:

- **Database Separation**: Uses three distinct connection pools (Read-only, Operator, Break-glass) to enforce the principle of least privilege.
- **Compliance Engine**: Generates Estonian VAT and Annual reports using real-time ledger data.
- **Operator Portal**: Internal API surface for device approvals, partition management, and system overrides.

## 🛠️ Tech Stack

- **Runtime**: Node.js 20+
- **Language**: TypeScript
- **Framework**: Express.js
- **Database**: PostgreSQL (multi-role access)
- **Email**: SMTP integration for device verification challenges.

## 🚀 Key Features

### Estonian Compliance
Automated generation of reports for Estonian e-government systems:
- **VAT Reports (KMD)**: Quarterly summaries of domestic, EU, and international sales.
- **KMD INF**: Automated attachment for transactions > 1000 EUR.
- **Annual Reports**: Full financial statements (Profit/Loss, Assets, Liabilities, Equity) derived from the ledger.
- **TSD Reports**: Payroll and tax declarations.
- **PDF Generation**: Professional, branded PDFs for all compliance documents.

### CEO & Business Analytics
Real-time monitoring of platform performance and financial health:
- **Core Metrics**: Impressions, Revenue, and Active Publisher tracking with WoW/DoD comparisons.
- **CEO Metrics**: Estimated Runway, Net Cash Flow (30d), Customer LTV, and Churn Rate.
- **Advanced Analytics**: Signup cohort retention analysis and linear growth revenue forecasting (6-month outlook).

### Accounting & Reconciliation
- **Bank Reconciliation**: Automated matching of bank statement entries (SEPA/Wise) against internal invoices and expenses.
- **Audit Logging**: Mandatory persistence of all operator actions to the `audit_logs` table with 7-year retention.
- **Impersonation**: Secure, JWT-based "one-click" impersonation for troubleshooting user issues.

### Infrastructure Management
- **Partition Control**: Automates PostgreSQL table partitioning for high-volume analytics data (ad_events, bid_logs, etc.).
- **Job Orchestration**: Real-time health monitoring of Autopilot background jobs and Migration Studio.
- **System Telemetry**: Unified view of App health, Database sessions, and Host metrics (disk/CPU/RAM).
- **Break-glass Access**: Highly audited emergency access for system-level remediation.

## 🚦 Getting Started

### Prerequisites
- PostgreSQL 16
- Access to the main platform database

### Installation
```bash
cd control-plane
npm install
```

### Development
```bash
npm run dev
```

## 🛡️ Security & Auditing

- **Audit Logging**: Every operator action is logged with actor ID, reason, and timestamp.
- **Device Approval**: New administrative devices require email-based verification challenges.
- **Role-Based Access**: Restricted to `apex_operator` and `apex_readonly` database users for standard operations.
