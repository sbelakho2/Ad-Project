+++
title = "Console analytics | Apex API reference"
description = "Revenue, lift, fees and data-quality views over the analytics store, with CSV export."
template = "api.html"
weight = 18

[extra]
h1 = "Console analytics"
og_image = "og/guides.png"
+++

## Purpose

The workspace's numbers from the analytics store: revenue with reporting
coverage, what changed and what is billable, the fee lifecycle, and data
quality. Each view exports as CSV.

## Method and path

- `GET /console/analytics?view={view}` renders one view. Views: `revenue`
  (default), `lift`, `fees` and `data-quality`.
- `GET /console/analytics?view={view}&format=csv` downloads the view as
  `revenue.csv`, `lift.csv`, `fees.csv` or `data-quality.csv`.
- `GET /console/analytics?view=fees&change={change_id}` opens one fee
  change's detail.
- `GET /console/analytics?...&platform={platform}` filters to
  `applovin_max`, `levelplay` or `admob`.

## Authorization

None on the page itself; the deployment must configure the analytics store.
The read window is the thirty complete days ending yesterday, shown on the
page with the as-of time.

## Request fields

All parameters are GET values, so every view is linkable and exportable.
Unknown views and unknown platform filters are 400 pages naming the allowed
set.

## States

- Revenue: daily revenue with its reporting coverage, breakdowns by
  platform, format and country, and the data-quality panel.
- Lift: each experiment with its observed lift, decision outcome and the
  billable bound when one exists, plus the data-quality panel.
- Fees: fee lines with state; with `change` selected, the single change's
  fee detail.
- Data quality: matched source days, missing or provisional days,
  deduplicated rows and stale sources, each with its reason when absent.
- Missing numbers render as `Not available` with the reason; a zero is only
  shown with a known denominator.

## Failure states

- 400 for an unreadable query, an unknown view or an unknown platform
  filter.
- 503 when the analytics store is not configured, with the reason on the
  page.
- A store that does not answer renders the same page with the honest
  unreachable state; it is never filled with invented numbers.

## No-JS contract

Views, filters and exports are GET links. The page is static HTML apart from
the requested read.

## Idempotency

Safe reads; the as-of time is part of the rendered context.

## Current deployment

Reads go through the analytics store's query layer with the deployment
currency; the projections that feed them are documented in the projections
guide.
