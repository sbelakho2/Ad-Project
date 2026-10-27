+++
title = "Console settings | Apex API reference"
description = "Account, theme, sessions, policy, monitoring and billing pages on the publisher console."
template = "api.html"
weight = 17

[extra]
h1 = "Console settings"
og_image = "og/guides.png"
+++

## Purpose

Workspace settings: the account record, the theme choice, the session list,
the recorded policy, the free monitoring health with per-platform checks, and the
billing plans with their cancel actions.

## Method and path

- `GET /console/settings` renders the account page.
- `POST /console/settings/theme` sets the theme and answers 303 back.
- `GET /console/settings/access` lists the live change-log sessions.
- `POST /console/settings/access/revoke` revokes one session by handle and
  answers 303 back.
- `GET /console/settings/policy` renders the recorded policy.
- `GET /console/settings/monitoring` renders the free monitoring state and each platform's
  reporting health.
  cancellation and answers 303 back. The plan bills the current period only;
  the cancelled record is kept with its stop period.
- `GET /console/settings/billing` renders the managed ad-ops plan, its
  written scope and its cancel action.
- `POST /console/settings/billing/managed/cancel` records the managed
  cancellation and answers 303 back, with the same stop semantics.

## Authorization and CSRF

Every settings page requires a live change-log session, and the theme,
revoke and cancel POSTs also require the hidden `csrf_token` field; without a session they 303-redirect to the
change-log access panel and a wrong token is 400. The read pages need no
session, but the access page only lists sessions when one is live.

## Request fields

- `POST /console/settings/theme`: `theme` is `auto`, `light` or `dark`.
- `POST /console/settings/access/revoke`: `handle` is the session handle
  shown on the access page.
  `POST /console/settings/billing/managed/cancel`: no fields beyond the
  token; a publisher with no recorded plan has nothing to cancel and gets
  400.

## States

- Account: the workspace identity, currency and plan facts.
- Theme: the choice is stored in a one-year cookie scoped to `/console`
  (HttpOnly, SameSite=Lax, Secure over HTTPS). An invalid value re-renders
  the page with the copy `Choose System, Light or Dark.` and answers 400.
- Access: each live session with its handle, open time and last use; the
  current session is marked.
- Policy: the recorded policy file's values and whether approval is
  required right now; without a readable policy the page says so.
- Monitoring: free with every connection; per-platform health, the last report date and alert delivery
  and one row per platform with its last report date and readiness.
- Billing: the managed ad-ops line, the written scope and the cancel action;
  nothing recorded renders the offering's price points without a cancel
  form.

## Failure states

- 400 for an invalid theme value, a bad handle, a wrong CSRF token or a
  cancel with no recorded plan.
- 303 to the access panel without a live session.
- 500 with a support reference for a connection store read failure or an
  unreadable plan state file.

## No-JS contract

Forms and links only. The theme is a cookie, not a script setting.

## Idempotency

Setting the same theme again is harmless; revoking an already-ended session
still answers 303 because the session is not live any more; cancelling an
already-cancelled plan keeps the recorded stop period unchanged.

## Current deployment

The policy page reads the deployment's policy file; monitoring and billing
read the recorded plan state files beside the connection records, and the
monitoring page also reads the snapshot dates. Monitoring has no plan tier, price, billing date or cancellation route.
