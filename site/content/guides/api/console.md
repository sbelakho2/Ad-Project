+++
title = "Console overview | Apex API reference"
description = "The account at a glance: latest audit snapshot and verified history totals, server-rendered."
template = "api.html"
weight = 9

[extra]
h1 = "Console overview"
og_image = "og/guides.png"
+++

## Purpose

The account at a glance: the latest audit snapshot and the verified change-log
totals. The console is server-rendered HTML with no script elements; every
mutation is a form POST that answers 303 See Other.

## Method and path

`GET /` on the console origin (`https://console.<domain>`). The console host
proxies its whole origin to the console service, so `/` is the overview and
the sibling pages live under `/console/*` on the same host. The marketing
host does not proxy `/console`.

- `GET /console/audit` and its export and finding pages,
- `GET /console/ledger` and its record pages,
- `GET /console/connections`, `/console/proposals/{id}`,
  `/console/experiments`, `/console/billing`, `/console/settings` and
  `/console/analytics`.

## Authorization

A live change-log session is required. Without one the request is
`303 See Other` to `/console/ledger` (the token-entry and sign-in panel).
The session cookie is host-only, `Path=/`, HttpOnly and SameSite=Lax; it is
issued by the one-time emailed sign-in link or the publisher-token exchange
and is never readable by script.

## Request fields

None.

## States

- First-run: no audit source and no publisher ledger configured. The page
  renders the introduction instead of zeroed panels.
- Latest audit: the newest snapshot across AppLovin MAX, LevelPlay and AdMob,
  with the capture date and finding counts by severity.
- History: the verified record total across all configured publisher ledgers.
  With no ledger configured the page says so; when every configured ledger is
  unreadable it names the unavailability reason; a partially readable set
  keeps the verified total and marks the unreadable count.

## Failure states

None at this page level. A read failure of one source surfaces as the
unavailable state for that source on the page; reads never fail the whole
page.

## No-JS contract

The page is complete HTML without JavaScript. Filters and navigation are
plain links; there is no client-side state to lose.

## Idempotency

A safe read; repeated loads render the same bytes while the sources are
unchanged.

## Current deployment

The page combines real snapshot and ledger sources; when a deployment has
neither, it shows the first-run state rather than invented numbers.
