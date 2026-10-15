+++
title = "Console settings | Apex API reference"
description = "Account, theme, sessions, policy and monitoring pages on the publisher console."
template = "api.html"
weight = 17

[extra]
h1 = "Console settings"
og_image = "og/guides.png"
+++

## Purpose

Workspace settings: the account record, the theme choice, the session list,
the recorded policy and the monitoring plan with per-platform health.

## Method and path

- `GET /console/settings` renders the account page.
- `POST /console/settings/theme` sets the theme and answers 303 back.
- `GET /console/settings/access` lists the live change-log sessions.
- `POST /console/settings/access/revoke` revokes one session by handle and
  answers 303 back.
- `GET /console/settings/policy` renders the recorded policy.
- `GET /console/settings/monitoring` renders the plan and each platform's
  reporting health.

## Authorization and CSRF

The theme and revoke POSTs require a live change-log session and the hidden
`csrf_token` field; without a session they 303-redirect to the change-log
access panel and a wrong token is 400. The read pages need no session, but
the access page only lists sessions when one is live.

## Request fields

- `POST /console/settings/theme`: `theme` is `auto`, `light` or `dark`.
- `POST /console/settings/access/revoke`: `handle` is the session handle
  shown on the access page.

## States

- Account: the workspace identity, currency and plan facts.
- Theme: the choice is stored in a one-year cookie scoped to `/console`
  (HttpOnly, SameSite=Lax, Secure over HTTPS). An invalid value re-renders
  the page with the copy `Choose System, Light or Dark.` and answers 400.
- Access: each live session with its handle, open time and last use; the
  current session is marked.
- Policy: the recorded policy file's values and whether approval is
  required right now; without a readable policy the page says so.
- Monitoring: the monitoring plan and one row per platform with its last
  report date and readiness.

## Failure states

- 400 for an invalid theme value, a bad handle or a wrong CSRF token.
- 303 to the access panel without a live session.
- 500 with a support reference for a connection store read failure.

## No-JS contract

Forms and links only. The theme is a cookie, not a script setting.

## Idempotency

Setting the same theme again is harmless; revoking an already-ended session
still answers 303 because the session is not live any more.

## Current deployment

The policy page reads the deployment's policy file; monitoring reads the
connection metadata and the snapshot dates.
