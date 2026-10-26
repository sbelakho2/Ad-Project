+++
title = "Apex API reference | Routes, authorization and errors"
description = "Routes, authorization and errors for the current Apex API."
sort_by = "weight"
template = "api-section.html"

[extra]
h1 = "Use the API"
lede = "Routes, authorization and errors."
og_image = "og/guides.png"
+++

| Endpoint | Purpose and authorization |
|---|---|
| [GET /healthz](/guides/api/healthz/) | 200 status JSON. Liveness endpoint. |
| [GET /metrics](/guides/api/metrics/) | Prometheus text: build, status counters, in-flight and uptime. |
| [GET /v1/audit/{platform}](/guides/api/audit-snapshot/) | Audit from configured snapshot; unknown platform 400, missing snapshot 404. |
| [POST /v1/audit/{platform}](/guides/api/live-audit/) | Live read-only audit using publisher body and deployment bearer authorization. Disabled deployment 503. |
| [GET /v1/ledger/{publisher}](/guides/api/ledger/) | Publisher bearer token; verified records only. 401, 422 and 503 are meaningful. |
| [POST /v1/cycle/{publisher}?platform=...](/guides/api/cycle/) | Validated optional policy. Current shipped runner reports read_only. |
| [POST /v1/leads](/guides/api/leads/) | Valid JSON or URL-encoded lead. Success is 202 received. |
| [POST /v1/mollie/webhook](/guides/api/mollie-webhook/) | Accept a Mollie payment id, fetch the payment server-side, and record it. |
| [Console overview](/guides/api/console/) | GET / on the console origin. Server-rendered HTML with no JavaScript. |
| [Console audit](/guides/api/console-audit/) | GET /console/audit, findings and export.csv; GET query values keep views linkable. |
| [Console ledger](/guides/api/console-ledger/) | Token exchange, verified records and the session end form. CSRF on every mutation. |
| [Console connections](/guides/api/console-connections/) | Credential submit, rotate and disconnect per platform; secrets never round-trip. |
| [Console proposal index](/guides/api/console-proposals-index/) | GET /console/proposals lists every proposal with its setting, creation time and decision state. |
| [Console proposals](/guides/api/console-proposals/) | Review a proposal and record an approve or decline decision with an idempotency key. |
| [Console experiments](/guides/api/console-experiments/) | GET-link filters and tabs, plus the incident recovery state. |
| [Console billing](/guides/api/console-billing/) | The fee register and one invoice period with its calculation. |
| [Console settings](/guides/api/console-settings/) | Account, theme, sessions, policy and monitoring pages. |
| [Console analytics](/guides/api/console-analytics/) | Revenue, lift, fees and data-quality views with CSV export. |
| [Free audit request](/guides/api/free-audit-request/) | POST /free-audit and GET /audit-requested/. Public form, 303 to a stable confirmation. |

## Contract note

Do not silently rename API fields. The console pages are documented next to
the JSON endpoints because they share the service; the projections guide
describes the read models behind the analytics views.
