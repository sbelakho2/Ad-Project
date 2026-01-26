# ApexMediation Documentation Index

Welcome to the master documentation index for the ApexMediation platform. This page provides a map to all technical and business documentation within the repository.

## 🏁 Getting Started
- **[Main README](../README.md)**: Project overview, architecture, and quick start.
- **[Development Guide](./Internal/Development/DEVELOPMENT.md)**: Detailed setup, workflow, and technology stack.
- **[Project Status](./Internal/Deployment/PROJECT_STATUS.md)**: Current delivery snapshot and readiness report.

## 🏗️ Architecture
- **[System Architecture](./Architecture/enhanced_ad_stack_srs_v2_0.md)**: High-level design and SRS.
- **[VRA Implementation Summary](./VRA_IMPLEMENTATION_SUMMARY.md)**: High-level summary of VRA fixes and features.
- **[VRA Technical Specification](./VRA_SPEC.md)**: Deep dive into VRA mechanics and data contracts.
- **[SSAI Improvements](./SSAI/SSAI_IMPROVEMENTS.md)**: Server-Side Ad Insertion details.

## 👨‍💻 Developer Resources
- **[Backend API](./Developer-Facing/README.md)**: API reference and integration guides.
- **[Autonomous Sales API](./Developer-Facing/AUTONOMOUS_SALES_API.md)**: API reference for the outbound sales automation system.
- **[SDK Integration](./Customer-Facing/00_OVERVIEW.md)**: Android, iOS, Unity, Web, and TV SDK docs.
- **[Testing Guide](./Internal/Development/TESTING_GUIDE.md)**: Strategies for unit, integration, and E2E testing.

## 🚀 Deployment & Operations
- **[Deployment Roadmap](./Internal/Deployment/DEPLOYMENT_ROADMAP.md)**: Production rollout strategy.
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

## 🤖 Sales Automation

> **⚠️ Architecture Note (January 2026):** All sales automation work runs on the **Test Server** (157.90.126.205:4001).
> The App Server control-plane is strictly a proxy client.

- **[Autonomous Sales V2](./Internal/Sales/AUTONOMOUS_SALES_V2.md)**: Zero-cost outbound prospecting with Thompson Sampling, Spintax, and Naive Bayes classification. Runs on Test Server.
- **[Autonomous Sales V2 Quick Ref](./Internal/Sales/AUTONOMOUS_SALES_V2_QUICK_REF.md)**: Quick reference for V2 operations.
- **[Closed-Loop System](./Internal/Sales/CLOSED_LOOP_SYSTEM.md)**: How webhooks automatically train the Thompson Sampler.
- **[Cialdini Sales Strategy](./Internal/Sales/CIALDINI_SALES_STRATEGY.md)**: Psychology-based trial nurture system (V1).
- **[Sales Automation Operations](./Internal/Sales/SALES_AUTOMATION_OPERATIONS.md)**: V1 operations guide.
- **[Sales Automation Summary](./Internal/Sales/SALES_AUTOMATION_SUMMARY.md)**: V1 implementation overview.

## �📊 Quality Assurance
- **[Live Testing Report](./LIVE_TESTING.md)**: Results from latest live execution cycles.
- **[User Journey Analysis](./USER_JOURNEY_ANALYSIS.md)**: UX and flow validation.
- **[Sandbox Execution](./Internal/QA/sandbox-execution/2025-12-08/PROGRESS.md)**: Progress tracking for sandbox verification.

---
*Last updated: 2026-01-25*
