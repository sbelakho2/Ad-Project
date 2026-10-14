# Adyen Dry-Run Evidence Log

_Last updated: 2025-11-25_

This folder tracks progress toward the end-to-end Adyen billing rehearsal described in
`docs/Internal/Deployment/ADYEN_COLLECTION_RUNBOOK.md`. Use it to capture commands, artifacts (PDFs,
`.eml` files, screenshots), and blockers so the `0.0.9 Billing, Usage & Adyen Sandbox Tests` gates can be
checked off with auditable evidence.

## 1. Prerequisites

| Item | Status | Notes |
| --- | --- | --- |
| Adyen CLI installed (`adyen version`) | ✅ | Installed v1.32.0 via GitHub release (`adyen version` now reports `adyen version 1.32.0`). |
| `STRIPE_SECRET_KEY` (test mode) | 🚧 | Still missing locally (`env | grep STRIPE` returned nothing). Need `sk_test_*` from Adyen Dashboard stored in Infisical + local `.env`. |
| `STRIPE_WEBHOOK_SECRET` | ⏳ | Will be populated after running `adyen listen ...`. |
| Test customer seeded in DB | ⏳ | Need `cus_xxx` saved on sandbox org once CLI+key are ready. |
| Evidence folder structure | ✅ | `docs/Internal/QA/adyen-dry-run/` created (this file). |

## 2. Execution Plan Snapshot

1. Install Adyen CLI (`adyen version` should report successfully).
2. Export `STRIPE_SECRET_KEY` (test mode) and optionally add to `.env.local` for backend scripts.
3. Run `adyen customers create --email test+adyen@apexmediation.ee` and patch the returned `cus_...` onto the sandbox org record.
4. Emit metered usage via `adyen billing/meter-events create ...`.
5. Create + finalize invoice (`adyen invoices create`, `adyen invoices pay`).
6. Tail backend logs/webhook processing to ensure status = `paid` and `stripe_invoice_id` persisted.
7. Download invoice PDF + capture billing console screenshots + Resend emails; drop assets into this folder.
8. Update checklist section `0.0.9` and `6. Invoice → Payment Dry-Run` with links to the collected evidence.

## 3. Current Blockers

- `STRIPE_SECRET_KEY` (test mode) not available locally. Need user-provided key or Infisical export before CLI commands can hit the API.
- `STRIPE_WEBHOOK_SECRET` cannot be generated until the CLI is authenticated (depends on the secret key/login above).

## 4. Next Actions Once Unblocked

- [ ] Install Adyen CLI and commit the version output to this log.
- [ ] Run `adyen login`/`adyen login --apikey` to persist credentials locally (or set env var).
- [ ] Execute steps 3–7 above, saving:
  - Customer creation command + JSON response (`customer.json`).
  - Meter event + invoice IDs.
  - Webhook log snippet proving `invoice.payment_succeeded` reached the backend.
  - Invoice PDF, email `.eml`, console screenshots.
- [ ] Mark checklist items `0.0.9` bullet 3/4 and Section 6 as completed with links to the saved artifacts.

---

Keep appending dated notes below as progress continues.

### 2025-11-25
- Installed Adyen CLI v1.32.0 (`~/.local/bin/adyen --version`).

```bash
$ ~/.local/bin/adyen --version
adyen version 1.32.0
```

- Confirmed environment still lacks `STRIPE_SECRET_KEY`; cannot run `adyen login` or API commands yet.
- Created this evidence log folder/file to capture future command output and artifacts.
