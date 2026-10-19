> Superseded. Describes a removed pre-pivot product. Current documentation: [docs index](../INDEX.md).

# Billing and Tiers (BYO-Only)

- Core rule: You never touch publisher network contracts or payouts. Apex bills a platform fee on gross mediated ad revenue (reconciled via network APIs/CSV uploads).
- Billing unit: Gross mediated ad revenue = sum of network payouts reported via API/uploads; never estimated revenue only.
- Tier evaluation: Use trailing 3-month average or current run rate (publisher-favorable).
- Discounts: Multi-year/exclusivity can lower %; never below infra/support floor.

## Tiers
- **Starter (0% up to $10k/app/mo)**
  - SDKs, Migration Studio, basic revenue/latency dashboards.
  - No transparency auctions, VRA reconciliation/exports, fraud dashboards, or A/B tools.
  - No card/bank required until upgrade.
- **Growth (2.5%)**
  - Everything in Starter plus transparency auctions, VRA reconciliation/exports, fraud/quality dashboards, and A/B/experiments.
  - Priority support; uptime SLA.
- **Scale (2.0% → 1.25% with annual prepay)**
  - Everything in Growth plus RBAC, BI/export pipelines, dedicated migration/integration help, and roadmap sessions.
- **Enterprise (1.0–1.5% custom)**
  - Everything in Scale plus custom SLAs, white-label options, bespoke data feeds, compliance support; invoice + wire after finance review (autopay default).

## Access Controls
- Starter is limited to basic dashboards. Transparency/VRA/fraud/experiments are gated to Growth+ in Console (Navigation, Transparency, VRA, Fraud pages).
- Billing Settings surfaces the tier capability grid; CTA to upgrade is available.

## Env/Secrets
- Console requires SMTP configuration (`SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASS`) for email via ApexMail, `NEXTAUTH_SECRET`, JWT secrets.
- Backend 2FA issuer is set via `TWOFA_ISSUER` to keep branding consistent.

## Website Alignment
- `/pricing` page and Console signup copy reflect the above tiers and gating.
