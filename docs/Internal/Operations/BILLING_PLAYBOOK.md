# Billing Playbook — Flows, Failure Modes, Dunning, Reconciliation, RACI

Last updated: 2025-11-12
Owner: Billing/Platform

## Scope
Operational handbook for the Billing subsystem: usage metering, invoicing, dunning, reconciliation, and on-call procedures.

## Core flows
- Usage metering (online)
  - Source: usage_events from backend services (billable subset)
  - Storage: Postgres (OLTP), ClickHouse (analytics rollups)
  - Monitoring: Prometheus counters; dashboards under monitoring/grafana/*
- Invoice lifecycle
  - Generate internal invoice snapshot → sync with Adyen (overages)
  - Serve PDFs via GET /billing/invoices/:id/pdf with ETag caching
- Reconciliation
  - Compare internal snapshot vs Adyen; tolerance 0.5%; persist diffs to billing_audit

## Failure modes and responses
- Adyen API errors
  - Retries with exponential backoff (1s→16s); idempotency keys per org+day
  - Persistent failures recorded to Redis for manual retry; counters exported
- Database errors
  - OLTP timeouts: respect statement timeouts; alert on latency/error rate; retry safe operations
- High error rate/latency
  - Use Admin → Health (RED tiles) to identify bottlenecks; check adapter timeout spikes

## Dunning policy (summary)
- Thresholds for overdue invoices; staged email notices; optional service degradation for severe delinquency
- Documented templates and escalation path (to be aligned with Legal)

## Reconciliation checklist
1. Trigger reconcile (Admin → Billing → Reconcile now) — sets Idempotency-Key
2. Review discrepancies in the returned payload and billing_audit
3. If >0.5% mismatch, file an incident and assign Engineering + Finance

## RACI
- Billing job reliability: Platform (R), SRE (A), Backend (C), Finance (I)
- Adyen credentials rotation: Platform (R), Security (A), Finance (C)
- Dunning templates/process: Finance (R), Legal (A), Platform (C)

## Runbooks
- Adyen outage
  - Switch to read-only billing views; queue writes for retry; post status in Console
- Retention purge
  - Use backend/scripts/purge-old-usage.ts (dry-run by default); schedule weekly; manual CONFIRM gate

## References
- API docs: docs/Backend/BILLING_API.md
- Keys rotation: docs/Internal/Operations/ADYEN_KEYS_ROTATION.md
- PCI scope: docs/Internal/Security/PCI_SCOPE.md
