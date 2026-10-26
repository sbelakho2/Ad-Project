# Security Review Register — Keys, Billing, PII

> **VERIFY-FIRST:** This register satisfies the Part 0 requirement for formal security reviews covering cryptographic keys, billing, and PII handling. Update the table whenever a review happens.

| Area | Scope | Last Review | Owner | Findings | Evidence |
| --- | --- | --- | --- | --- | --- |
| Cryptographic Keys | Ed25519 transparency signer, JWT secrets, service-to-service tokens. | 2025-11-12 | Security Eng | Rotation automation drafted; need quarterly reminder workflow. | `docs/Internal/Security/COMPREHENSIVE_AUDIT_REPORT.md` §7, `.github/workflows/security-trivy.yml` |
| Billing Systems | Usage metering, invoicing, Mollie integration, document retention. | 2025-11-15 | Billing Eng Lead | Mollie dunning retries stable; add monitoring for FX source failures. | `docs/Internal/Operations/BILLING_PLAYBOOK.md`, `backend/scripts/cron-jobs.ts` |
| PII Handling | Privacy exports/deletes, SDK consent capture, log redaction. | 2025-11-16 | Privacy Eng / DPO | Need automated audit log linking DSR requests to deletions. | `docs/Internal/Security/DATA_MAP.md`, `backend/src/routes/privacy.routes.ts` |

## Review Workflow

1. **Schedule** quarterly reviews per area (calendar invites linked to PagerDuty maintenance windows).
2. **Collect Evidence** (runbooks, PR links, test outputs) and attach to the review ticket.
3. **Record Findings** above and in `docs/Internal/Development/FIXES.md` with FIX IDs.
4. **Verify Remediation** before closing the review.

## Upcoming Reviews

- 2025-12-15 — Cryptographic key manifest signing (link to ADR once opened).
- 2026-01-05 — Billing model audit for BYO accounts.
- 2026-01-12 — Privacy DSR automation dry-run.
