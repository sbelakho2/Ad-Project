+++
title = "Free audit request | Apex API reference"
description = "The marketing form POST and the confirmation page: fields, validation copy and dedup."
template = "api.html"
weight = 19

[extra]
h1 = "Free audit request"
og_image = "og/guides.png"
+++

## Purpose

The free-audit request: a marketing form POSTs to the API, the request is
stored append-only, and the browser lands on a confirmation page with a
stable request reference.

## Method and path

- `GET /free-audit` answers 303 to `/free-audit/` (the form page lives at the
  canonical trailing-slash address).
- `GET /free-audit/` renders the request form.
- `POST /free-audit` validates and stores the request, then answers 303 to
  `/audit-requested/?ref={request_id}`.
- `GET /audit-requested/` renders the confirmation; `ref` is shown when it
  matches the request-id shape (`req_` plus ten hex characters).

## Authorization

None. The form is public; the API stores the request and forwards it to the
deployment's mail webhook when one is configured. A lead with an invalid
shape is refused before anything is stored.

## Request fields

Form body (`application/x-www-form-urlencoded`):

- `email` (required)
- `studio` (required): the publisher or studio name.
- `platforms` (at least one): `max`, `levelplay`, `admob`; `applovin_max` is
  accepted as the MAX alias. A repeated form key joins its values.
- `revenue` (optional): one of the exact bands, or its marketing slug
  (`under-10k`, `10k-50k`, `50k-100k`, `100k-300k`, `over-300k`).
- `notes` (optional, up to 2,000 characters).

## States

- Fresh form, field errors with the exact copy and safe values preserved,
  submission-failure state with the retry action, and the confirmation page.
- A validation failure answers 400 with the form; a storage failure answers
  503 or 500 with the submission-failure state.
- Identical retries inside 24 hours deduplicate to the original request id.

## Failure states

- 400 with the field errors on the form.
- 503 with the submission-failure state when no lead store is configured.
- 500 with the submission-failure state when the store fails while writing.

## No-JS contract

A plain form POST and a 303 redirect; the confirmation is a stable GET page.

## Idempotency

The request id is derived from the submitted fields, so a retry of the same
request keeps one stored lead and one reference.

## Current deployment

The stored lead is append-only; the confirmation page shows the reference so
support can find it without exposing any field values.
