# VRA Console UI Smoke Notes (Read‑Only Verification)

Purpose
- Lightweight manual checks to ensure the Console’s VRA surfaces render correctly and preserve filters, without any backend writes.
- Keep this document updated during canary/pilot and after notable UI changes.

Environments
- Console URL: https://console.apexmediation.ee (or local dev)
- API URL: https://api.apexmediation.ee (or local dev)
- Flags expected for canary: `VRA_ENABLED=true`, `VRA_SHADOW_ONLY=true`

Preconditions
- Backend reachable; user authenticated in Console.
- Optional: recent Overview call so pilot‑gate gauges are populated in `/metrics`.

Checklist (Deltas)
- [ ] Filters persist into the CSV link (kind, min confidence, window, pagination)
- [ ] CSV downloads with 8 columns
- [ ] `amount` has 6 decimal places; `confidence` has 2 decimal places
- [ ] `reason_code` is sanitized (no raw emails, tokens, long numerics; commas/newlines/tabs removed; quotes escaped)
- [ ] Pagination works; header row remains stable; no column drift across pages

Checklist (Overview)
- [ ] By‑network cards respect the current time window
- [ ] Deep‑linking to a specific network preserves the active window
- [ ] Tooltip labels render; a11y of tooltips (keyboard focus, escape to close) is intact
- [ ] Confidence badges render with expected colors and labels

Accessibility & UX
- [ ] Keyboard navigation covers all interactive elements on Deltas and Overview panels
- [ ] Focus outlines visible; no trapped focus in tooltips or modals
- [ ] Color contrast acceptable for badges and small text

Observability quick glance (optional)
- [ ] After viewing Overview: `/metrics` contains `vra_coverage_percent` and `vra_variance_percent`
- [ ] Grafana panels for Matching and Reconcile update without errors; runbook links open correct anchors

Known caveats / TODOs
- [ ] (add here)

Session log (example)
```
Date: 2025-11-25
Env: staging
User: ops@example.com

Deltas
- Filters: kind=underpay, min_conf=0.50, from=2025-11-01, to=2025-11-02
- CSV: 8 columns OK; amount/conf formatting OK; reason_code sanitized (quotes escaped)

Overview
- By‑network cards reflect window; deep‑link retains window
- Tooltips a11y: OK (keyboard focus enters/leaves; Esc closes)

Observability
- `/metrics` has gauges after Overview
- Grafana dashboards show timeseries; links to Runbook anchors OK
```
