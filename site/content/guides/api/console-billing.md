+++
title = "Console billing | Apex API reference"
description = "The fee register and one invoice period with its calculation on the publisher console."
template = "api.html"
weight = 16

[extra]
h1 = "Console billing"
og_image = "og/guides.png"
+++

## Purpose

The billing surface: the register of closed periods with their fee state and
collection status, and one invoice period with the fee calculation.

## Method and path

- `GET /console/billing` renders the register.
- `GET /console/billing/invoices/{period}` renders one period
  (`YYYY-MM`).

## Authorization

A live change-log session is required; without one the request is `303 See
Other` to `/console/ledger`. The pages read the verified change log and the
publisher's own collection events (fee references naming another publisher
are never shown); they mutate nothing.

## Request fields

The period is a path value of the form `YYYY-MM`. Anything else is 404.

## States

- Register: the current month's lift fees, monitoring fee and payment
  status as cards, plus one row per recorded period linking to its invoice.
- Invoice: per lift line, the eligible scope revenue, the billable lower
  bound, the attributed increment, the agreed fee rate and the lift fee
  before tax; then the VAT section, the recorded payments and the billing
  changes of the period. Without a recorded VAT rate, the VAT row says
  `Not available` with the reason.
- Amounts are integer micros with an explicit currency; a missing amount is
  `Not available` with its reason, never zero.

## Failure states

- 404 for an unknown period or a value that is not a period.
- 404 when the change log does not exist; 422 when it does not verify.
- 500 with a support reference for a read failure.
- 503 when the workspace publisher is not configured.

## No-JS contract

Register rows link to the period pages; no client-side state.

## Idempotency

Safe reads.

## Current deployment

The register combines the verified change log with the deployment's
collection log; collection states come from the Adyen webhook processing.
