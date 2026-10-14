# Adyen Keys Rotation Runbook

Last updated: 2025-11-12
Owner: Platform/Operations

Scope
- Rotate Adyen API secret keys and webhook signing secrets without downtime.

Pre‑checks
- Confirm current usage of `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET` in environments (dev/staging/prod).
- Ensure `STRIPE_API_VERSION` is pinned (see backend/scripts/adyen-daily-usage-sync.ts).

Procedure (per environment)
1. In Adyen Dashboard, create a new Restricted API key with least‑privilege for Billing (meter events, invoices).
2. Add the new key to the environment secret store:
   - GitHub Environments → {ENV} → Secrets → `STRIPE_SECRET_KEY`
   - Kubernetes/Helm: values or secret manager (avoid plaintext in repos)
3. Update webhook endpoint signing secret (if regenerating):
   - Adyen Dashboard → Developers → Webhooks → Select endpoint → Reveal/Rotate signing secret
   - Store as `STRIPE_WEBHOOK_SECRET` in secret store
4. Deploy config change to dev → staging. Confirm:
   - Webhook verification succeeds (200 responses, no signature errors)
   - `adyen-daily-usage-sync` job completes (success counters increment)
   - Reconcile endpoint works against test mode
5. Flip production secrets during a quiet window. Verify health checks and logs.
6. Revoke old keys after 24–48h observation window.

Validation
- Metrics: `stripe_usage_sync_success_total` increases; `stripe_usage_sync_failures_total{reason}` steady/0.
- Webhook logs show `verified=true`, no 4xx/5xx spikes.

Rollback
- Revert to previous secrets in environment store; redeploy.

Notes
- Keys must not be committed to VCS. Secret scanning is enabled in CI.
- Keep API version string stable across rotations.
