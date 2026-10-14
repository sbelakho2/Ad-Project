# PCI Scope — Billing (Adyen)

_Last updated: 2025-11-18_  
_Owner: Security Team / Platform Engineering_  
_Review Cycle: Annually or when architecture changes (next review: 2026-11-18)_  
_Status: Active - SAQ-A eligibility maintained_

> **FIX-10 governance:** This document defines PCI compliance scope. Review required whenever payment architecture changes. For current deployment status, see `docs/Internal/Deployment/PROJECT_STATUS.md`.

Summary
- We do not handle raw cardholder data (CHD) on our servers or in our Console.
- All payment collection and management is delegated to Adyen-hosted surfaces (Adyen Elements/Checkout and Customer Portal).
- Target PCI scope: SAQ-A.

Key points
- Console Billing Settings links users to the Adyen Customer Portal; no card forms are embedded in our domain.
- Webhooks from Adyen are signature-verified and contain no sensitive PAN data; we persist only metadata/ids.
- Secrets (Adyen keys) are stored in environment/secret store and rotated per runbook.
- Logs and audit tables are redacted of PII and do not store full card details.

Controls
- TLS 1.2+ everywhere; HSTS enabled in production.
- Security headers middleware sets CSP, Referrer-Policy, Permissions-Policy, X-Content-Type-Options.
- Admin actions audited to `billing_audit` with actor attribution.

Evidence
- Console Settings → Adyen Portal deep link.
- Middleware: `backend/src/middleware/securityHeaders.ts`.
- Webhooks: `backend/src/routes/webhooks.routes.ts`.

Gaps/Notes
- Periodically review Adyen’s guidance for hosted payment pages to ensure continued SAQ-A eligibility.
- Ensure marketing website also avoids embedding card forms.
