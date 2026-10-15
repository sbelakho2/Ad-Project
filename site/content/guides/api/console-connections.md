+++
title = "Console connections | Apex API reference"
description = "Platform credentials on the publisher console: submit, rotate and disconnect with CSRF."
template = "api.html"
weight = 12

[extra]
h1 = "Console connections"
og_image = "og/guides.png"
+++

## Purpose

The workspace's platform credentials: see each platform's credential state,
submit or rotate a credential bundle, or disconnect a platform.

## Method and path

- `GET /console/connections` lists AppLovin MAX, LevelPlay and AdMob with
  their credential state, write label and last report date.
- `GET /console/connections/{platform}` renders one platform with its form
  and the running-work warning.
- `POST /console/connections/{platform}/submit` stores a new credential
  bundle and answers 303 back to the platform page.
- `POST /console/connections/{platform}/rotate` runs the same upsert with a
  new bundle.
- `POST /console/connections/{platform}/disconnect` removes the stored
  credentials and answers 303 back to the platform page.

## Authorization and CSRF

Every mutation needs a live change-log session; without one the request is
303-redirected to the change-log access panel. The hidden `csrf_token` field
is required and a wrong token is 400. Secrets never round-trip into HTML:
refused submissions preserve `account_id` only, never `api_key` or
`refresh_token`.

## Request fields

Per platform (`account_id` and `api_key` always required):

| platform | required | optional |
|---|---|---|
| `applovin_max` | `account_id`, `api_key` | `report_key` |
| `levelplay` | `account_id`, `api_key`, `refresh_token` | |
| `admob` | `account_id`, `api_key` | |

## States

- Credential states: unvalidated, validated, rejected, expired, absent.
- A submission stores the bundle and, where the deployment can validate,
  records what the live read actually did. With no verified platform host in
  the deployment the previously recorded state is kept, and a submission
  never invents a validated state.
- Disconnect warns when running experiments exist for the platform; when the
  change log is unreadable there, the warning says the check could not run.

## Failure states

- 400 for missing required fields (the form names each field and preserves
  safe values) or a wrong CSRF token.
- 303 to the change-log access panel without a live session.
- 404 for an unknown platform.
- 503 when the workspace publisher is not configured or the vault is not
  configured.

## No-JS contract

Forms and links only; every mutation answers 303 and the redirect target is
the stable platform page.

## Idempotency

Submit and rotate are upserts keyed by publisher and platform; repeating a
submission keeps one bundle.

## Current deployment

Credentials are stored through the deployment vault; validation runs only
where a live read path is configured.
