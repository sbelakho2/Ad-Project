# ApexMediation documentation index

The current product is the no-SDK revenue-operations stack in `rust/`, the
Zola marketing site in `site/`, and the Caddy/systemd deployment in
`infrastructure/`. Start with the root `README.md` and the infrastructure
runbook; everything else here supports those.

## Start here

- [Repository overview](../README.md): crates, commands, gates.
- [Infrastructure runbook](../infrastructure/README.md): hosts, services,
  environment files (`infrastructure/env/*.sample`), deployment.
- [Edge routing contract](../infrastructure/caddy/README.md): the three public
  hosts and the loopback-only operator plane.
- [Local CI](Internal/CI/LOCAL_CI.md): the pinned Woodpecker lanes.
- [Development guide](Internal/Development/DEVELOPMENT.md): toolchains and
  the Rust workspace.

## Publisher console and sign-in

- [Session and single sign-on](Internal/Console/SESSION_AUTH.md): the
  one-time-link journey, public hosts, cookie attributes, CSRF and token
  exchange.
- [Console security checklist](Internal/Console/SECURITY_CHECKLIST.md).
- [Console RBAC notes](Internal/Console/RBAC_SYSTEM.md).
- Public API guides live in `site/content/guides/api/` (rendered at
  `/guides/api/`).

## Operator control plane

- [Control-plane architecture](Internal/Architecture/System_Architecture.md).
- [Design system status](Internal/Architecture/DESIGN_SYSTEM_IMPLEMENTATION_STATUS.md).
- [Component reference](Internal/Architecture/COMPONENT_REFERENCE_GUIDE.md).

## Revenue engine

- [Apex sales engine](Internal/Sales/APEX_SALES_ENGINE.md): the owner-only
  outbound engine in `rust/crates/apex-sales`.
- [Setup Watch](Internal/Watch/APEX_WATCH.md): the paid daily app-ads.txt
  watch and its billing.
- [Adyen collection runbook](Internal/Deployment/ADYEN_COLLECTION_RUNBOOK.md).
- [Billing policy rollout](Internal/Deployment/BILLING_POLICY_ROLLOUT.md).

## Operations and security

- [Production readiness](Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md).
- [PITR recovery](Internal/Architecture/PITR_RECOVERY.md).
- [Security audit report](Internal/Security/AUDIT_REPORT.md).
- [Compliance status](Internal/Security/COMPLIANCE_STATUS.md).
- [Secrets management](Internal/Security/SECRETS_MANAGEMENT_SETUP.md).
- [Data retention](DATA_RETENTION_POLICIES.md).

## Historical documents

The entries under `docs/SSAI/`, `docs/Customer-Facing/`, `docs/Developer-Facing/`,
`docs/VRA_*`, `docs/Internal/Features/MigrationStudio/`, and the sales V1/V2
documents describe the removed SDK/SSAI/TypeScript product. They are kept for
the record and are not an operating guide for this repository. Superseded
pages carry a banner at the top.

- [Archived website/dashboard auth design](Internal/Architecture/WEBSITE_DASHBOARD_AUTH_INTEGRATION.md)
  (superseded by the session document above).
