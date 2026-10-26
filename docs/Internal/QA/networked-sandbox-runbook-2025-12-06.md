### Networked Sandbox Runbook (2025-12-06)

This runbook executes checklist sections 0.0.7–0.0.12 on staging, captures evidence, and produces an exportable bundle.

Prerequisites
- Staging console and API up; HSTS and `/ready` verified (see CHANGELOG 2025‑12‑06).
- Test org in staging; test email inbox; Mollie test keys; Resend sandbox.
- Sandbox apps built (iOS, tvOS, Android, Android TV, Unity, Web) pointing to staging.

Evidence bundle structure
```
evidence-2025-12-06/
  console/
  website/
  billing/
  vra/
  cron/
  soak/
  sdk-logs/
```

0.0.7 Console & Dashboard
1) Auth flows (signup/login/reset)
   - Create test user → verify email via Resend → login/logout → password reset
   - Save screenshots: `console/*.png`
2) Org/apps/API keys/placements
   - Create org; create two apps; generate API keys
   - CRUD placements; verify constraints; export JSON of placements
3) Enable FakeNetworkA/B/C per placement
   - Confirm secrets never appear in logs
4) Mediation Debugger
   - From sandbox apps, open debugger; capture timelines/no‑bid reasons
   - Verify filters/pagination; confirm PII redaction (hashes for IDs)
5) Auction logs/exports
   - Export hashed payloads per request; attach CSV/JSON to `console/`

0.0.8 Website
1) Security headers unit tests
   ```bash
   npm --prefix website run test
   ```
   - Save test output to `website/security-tests.txt`
2) Responsive sweep
   - Desktop/tablet/360px mobile screenshots → `website/responsive/*.png`
3) SEO/canonical/404 + broken links
   - Record canonical, meta, and 404 page screenshots; run link checker and save logs

0.0.9 Billing & Mollie
1) Synthetic usage
   - Starter (<$10k), Growth (~$50k), Scale (~$150k) sample usage; run aggregation; capture tier assignments
2) Cap transition
   - Exceed Starter cap → confirm `requires_payment_method` flips and console shows upgrade prompts
3) Mollie test customer
   - Create customer, add card via Customer Portal, send metered events, finalize invoice
   - Verify webhook transitions: open → paid; download PDF (shows Bel Consulting OÜ + Wise SEPA/ACH text)
4) Emails
   - “Upcoming invoice”, “invoice paid”, “payment failed” previews; save Resend logs
5) Dunning
   - Simulate failure; run dunning job; verify console status and notifications

0.0.10 VRA / Reconciliation
1) Traffic
   - Run multi‑day sandbox impressions/clicks/paid events
2) Statements
   - Create matching, under‑reported, and FX‑mismatched statements
3) Pipeline
   - Execute VRA; verify `recon_*` tables classifications
4) Console
   - Screenshots of deltas filtering (app/network/date) and dispute‑kit ZIP
5) Proof‑of‑Revenue
   - Generate daily roots/monthly digest; verify hashes match

0.0.11 Cron & Workers
1) Start staging cron/worker stack
   - Confirm zero crashes; capture logs; verify alerts for failures
2) Emails
   - Trigger sample notifications; confirm delivery in test inbox

0.0.12 Soak (End‑to‑End)
1) Traffic
   - Drive 1–5 RPS `loadAd` and reporting for ≥1 hour across sandbox apps (k6/Gatling/script)
2) Targets
   - Keep API p95 within targets, error rate <1%, low Sentry noise, stable CPU/memory on staging
3) Artifacts
   - Save k6/Gatling reports and system metrics to `soak/`

SDK logs and adapter evidence
- iOS/tvOS: capture simulator/device logs; include adapter picker runs and whitelist evidence.
- Android/TV: `adb logcat` with adapter spans; include connected test artifacts.
- Unity: batchmode logs and any native bridge traces.

Submission
- Zip `evidence-2025-12-06/` and link from `docs/Internal/Deployment/PRODUCTION_READINESS_CHECKLIST.md` corresponding sections.
