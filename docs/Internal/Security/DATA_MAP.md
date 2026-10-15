Data Map — Billing and Privacy

Purpose
This document enumerates personal and business data processed by the Billing subsystem, storage locations, processors, retention windows, and data subject rights mechanisms (export/delete).

Processors
- Adyen: Payments, invoices, Customer Portal. No PAN/BIN/CVV processed server‑side (SAQ‑A via Adyen Elements/Portal).
- Email (Postfix/ApexMail): Dunning and billing notifications.

Storage locations
- Postgres: `usage_events`, `billing_audit`, and related tables.
- ClickHouse: Aggregated usage metrics.
- Object storage (S3/GCS or local): Privacy export bundles under `privacy-exports/<tenant>/<user>/<ts>.jsonl`.

Data Subject Rights (DSR)
- Export: `POST /api/v1/privacy/export` enqueues export job (tenant + user scoped). Worker writes export bundle to configured provider (local/S3/GCS). Config: `PRIVACY_EXPORT_PROVIDER`, `PRIVACY_EXPORT_BUCKET`, `PRIVACY_EXPIRY_DAYS`.
- Delete: `POST /api/v1/privacy/delete` enqueues delete job to redact PII from Postgres tables for the tenant/user.
- Evidence: `backend/src/routes/privacy.routes.ts`, `backend/src/queues/processors/privacy.ts`, tests in `backend/src/routes/__tests__/privacy.*.test.ts`.

Retention
- Usage events and audit logs: default 18 months (configurable). Enforced by daily cron purge and supported by indexes.
- Config: `USAGE_RETENTION_MONTHS` (cron), `RETENTION_MONTHS` (manual script). Evidence: `backend/scripts/cron-jobs.ts`, migration `backend/migrations/postgres/20251112_023000_usage_retention_indexes.up.sql`.

Security
- Secrets stored via secret store (Infisical/GitHub Environments). Adyen key rotation runbook: `docs/Internal/Operations/ADYEN_KEYS_ROTATION.md`.
- Logs redact PII and secrets. Evidence: `backend/src/utils/logger.ts`, tests `backend/src/utils/__tests__/logger.redaction.test.ts`.
