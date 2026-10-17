# ApexMediation Documentation Index

Welcome to the master documentation index for the ApexMediation platform. This page provides a map to all technical and business documentation within the repository.

## 🏁 Getting Started
- **[Main README](../README.md)**: Project overview, architecture, and quick start.
- **[Development Guide](./Internal/Development/DEVELOPMENT.md)**: Detailed setup, workflow, and technology stack.
- **[Project Status](./Internal/Deployment/PROJECT_STATUS.md)**: Current delivery snapshot and readiness report.

## 🏗️ Architecture
- **[System Architecture](./Internal/Architecture/enhanced_ad_stack_srs_v2_0.md)**: High-level design and SRS.
- **[Migration Studio](./Internal/Features/MigrationStudio/README.md)**: Side-by-side migration simulator and cryptographically signed lift reports.
- **[VRA Implementation Summary](./VRA_IMPLEMENTATION_SUMMARY.md)**: High-level summary of VRA fixes and features.
- **[VRA Technical Specification](./VRA_SPEC.md)**: Deep dive into VRA mechanics and data contracts.
- **[SSAI Improvements](./SSAI/SSAI_IMPROVEMENTS.md)**: Server-Side Ad Insertion details.
- **[VAST/VMAP Parser](./SSAI/VAST_VMAP_PARSER.md)**: ⭐ NEW - IAB VAST/VMAP parsing module documentation.

## 👨‍💻 Developer Resources
- **[Backend API](./Developer-Facing/README.md)**: API reference and integration guides.
- **[Autonomous Sales API](./Developer-Facing/AUTONOMOUS_SALES_API.md)**: API reference for the outbound sales automation system.
- **[SDK Integration](./Customer-Facing/00_OVERVIEW.md)**: Android, iOS, Unity, Web, and TV SDK docs.
- **[Testing Guide](./Internal/Development/TESTING_GUIDE.md)**: Strategies for unit, integration, and E2E testing.

## 🚀 Deployment & Operations
- **[Deployment Roadmap](./Internal/Deployment/DEPLOYMENT_ROADMAP.md)**: Production rollout strategy.
- **[Local CI](./Internal/CI/LOCAL_CI.md)**: Run the Woodpecker lanes locally and ship release binaries with rsync.
- **[Control Plane Guide](./Internal/Operations/CONTROL_PLANE_GUIDE.md)**: Manual for operator tasks, compliance, and analytics.
- **[Production Readiness](./Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md)**: Pre-flight checklist for live environments.
- **[Monitoring Infrastructure](./Monitoring/MONITORING_INFRASTRUCTURE.md)**: Prometheus, Grafana, and alerting setup.
- **[Runbooks](./runbooks/)**: Operational procedures and incident response.
  - **[Sales Automation Operations](./runbooks/SALES_AUTOMATION_OPERATIONS.md)**: ⭐ NEW - Test Server sales-api operations runbook.
  - **[Rollback](./runbooks/ROLLBACK.md)**: Rollback procedures.

## ⚖️ Compliance & Business
- **[Estonian E-Systems](./Internal/Deployment/ESTONIAN_E_SYSTEMS.md)**: Integration with e-government services.
- **[Data Retention](./DATA_RETENTION_POLICIES.md)**: GDPR and local compliance policies.
- **[Billing & Tiers](./Developer-Facing/Billing-and-Tiers.md)**: Revenue share models and platform tiers.

## 📧 Email System (ApexMail)

> **⚠️ Architecture Note (February 2026):** ApexMail is the self-hosted email system.  
> It uses direct SMTP via Postfix on mail.apexmediation.ee with DKIM signing.

- **[ApexMail Design](./Internal/APEXMAIL_DESIGN.md)**: Self-hosted email infrastructure design and database schema.
- **[Self-Hosted SMTP Setup](./Internal/Sales/SETUP_SELF_HOSTED_SMTP.md)**: SMTP relay configuration guide.

## 🤖 Sales Automation

- **[Apex Sales Engine](./Internal/Sales/APEX_SALES_ENGINE.md)**: Current runbook for the owner-only outbound engine in `rust/crates/apex-sales`, which finds publishers and emails them.

## ⌚ Setup Watch

- **[Setup Watch](./Internal/Watch/APEX_WATCH.md)**: Runbook for the paid daily app-ads.txt watch in `rust/crates/apex-watch`: sign-up, the daily check, the guard, alerts, billing and cancel.

The entries below describe the removed TypeScript engine and its Test Server deployment; they are kept as history.

- **[Autonomous Sales V2](./Internal/Sales/AUTONOMOUS_SALES_V2.md)**: Zero-cost outbound prospecting with Thompson Sampling, Spintax, and Naive Bayes classification. Runs on Test Server.
- **[Autonomous Sales V2 Quick Ref](./Internal/Sales/AUTONOMOUS_SALES_V2_QUICK_REF.md)**: Quick reference for V2 operations.
- **[Closed-Loop System](./Internal/Sales/CLOSED_LOOP_SYSTEM.md)**: How webhooks automatically train the Thompson Sampler.
- **[Cialdini Sales Strategy](./Internal/Sales/CIALDINI_SALES_STRATEGY.md)**: Psychology-based trial nurture system (V1).
- **[Sales Automation Operations](./Internal/Sales/SALES_AUTOMATION_OPERATIONS.md)**: V1 operations guide.
- **[Sales Automation Summary](./Internal/Sales/SALES_AUTOMATION_SUMMARY.md)**: V1 implementation overview.

## 📊 Quality Assurance
- **[Live Testing Report](./LIVE_TESTING.md)**: Results from latest live execution cycles.
- **[User Journey Analysis](./USER_JOURNEY_ANALYSIS.md)**: UX and flow validation.
- **[Sandbox Execution](./Internal/QA/sandbox-execution/2025-12-08/PROGRESS.md)**: Progress tracking for sandbox verification.

## 🔒 Security
- **[VAST/VMAP Parser Security](./SSAI/VAST_VMAP_PARSER.md#security-features)**: XSS, XXE, DoS protections in ad parsing.
- **[Compliance Status](./Internal/Security/COMPLIANCE_STATUS.md)**: SOC 2, GDPR, PCI DSS compliance tracking.
- **[Security Audit Report](./Internal/Security/AUDIT_REPORT.md)**: Latest security audit findings.

---
*Last updated: 2026-02-01*
