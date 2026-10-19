+++
title = "Console ledger | Apex API reference"
description = "The change log on the publisher console: bearer-token exchange, verified records, record detail."
template = "api.html"
weight = 11

[extra]
h1 = "Console ledger"
og_image = "og/guides.png"
+++

## Purpose

The publisher's change log: open an access session with the publisher's
ledger token, read the verified records, inspect one record, or end the
session.

## Method and path

- `GET /console/ledger` renders the access panel without a live session, or
  the verified record list with one.
- `POST /console/ledger` exchanges the publisher name and token for an
  opaque session and answers 303 to `GET /console/ledger`.
- `GET /console/ledger/records/{seq}` renders one record.
- `POST /console/ledger/session/end` clears the session and answers 303 to
  `GET /console/ledger`.

## Authorization and CSRF

The token is submitted once in the POST body, never in a URL, and is never
stored or reflected; the cookie carries only an opaque session id. The
session cookie is `apex_ledger_session`: HttpOnly, SameSite=Lax, host-only
and scoped to `/` (the console origin is its own host), marked Secure on the
HTTPS console origin, and short-lived with a rolling window. Mutations require the hidden
`csrf_token` field; a wrong token is 400.

## Request fields

`POST /console/ledger` form fields:

- `publisher`: the machine-safe name (`^[A-Za-z0-9_-]{1,64}$`).
- `token`: the publisher's bearer token.

`POST /console/ledger/session/end` form field: `csrf_token`.

## States

- Access panel: a fresh panel, an invalid-access refusal with the publisher
  name preserved, or an expired-session notice.
- Records: verified records only, in chain order, with the chain envelope
  (`seq`, time, platform, previous hash, hash).
- Record detail: the full record and its place in the chain.

## Failure states

- 400 for an invalid publisher name or malformed form; the safe publisher
  value is preserved in the panel.
- 401 for a wrong token.
- 404 when no ledger file exists for the publisher.
- 422 when the chain does not verify, with the failing line; nothing is
  served from an unverified ledger.
- 503 `ledger_auth_not_configured` when the publisher has no configured
  token; without the token file no ledger is readable over HTTP.
- 303 to the change log when a record page or mutation is opened without a
  live session; the panel is the recovery point.

## No-JS contract

The change log works with forms and links only. The session is a cookie, not
client-side state.

## Idempotency

- Opening a session again mints a new session; the token is not stored.
- Ending an already-ended session still answers 303.
- Record reads are safe.

## Current deployment

The ledger is read through the verifier; a failed verification is never
papered over.
