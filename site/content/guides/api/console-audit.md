+++
title = "Console audit | Apex API reference"
description = "Audit form, report, finding detail and CSV export on the publisher console."
template = "api.html"
weight = 10

[extra]
h1 = "Console audit"
og_image = "og/guides.png"
+++

## Purpose

The publisher's audit surface: choose a platform, read the report, open one
finding, or export the report as CSV.

## Method and path

- `GET /console/audit` renders the platform form.
- `GET /console/audit?platform={platform}` renders the report for
  `applovin_max`, `levelplay` or `admob`.
- `GET /console/audit/findings/{id}` renders one finding by its stable id
  (`f_` plus twelve hex characters).
- `GET /console/audit/export.csv?platform={platform}` downloads the report as
  RFC 4180 CSV (`audit-{platform}.csv`).

## Authorization

A live change-log session is required; without one the request is `303 See
Other` to `/console/ledger`. The audit reads the deployment's stored snapshot
and never writes to a platform from these pages.

## Request fields

The platform is a GET query value, so the report page is linkable and
bookmarkable. There are no POST forms on these routes.

## States

- Form: platform choice; unknown platform re-renders the form with the field
  error naming the value.
- Report: findings sorted by severity, each row linking to its finding page.
  Money renders through the deployment currency; an estimate whose evidence
  lacks a money model says `Not estimated` with the rule-level note.
- Finding detail: evidence, affected scope and the estimate state.
- CSV: one row per finding. Columns: severity, rule, platform, subject,
  finding id, message, recovery model, estimated monthly recovery in micros
  with its currency, capture time, snapshot digest and generator.

## Failure states

- 400 on `/console/audit` for an unreadable query or an unknown platform; the
  form explains the problem and preserves nothing unsafe.
- 400 on `/console/audit/export.csv` without a platform ("A platform
  parameter is required to export a report.") or with an unknown platform.
- 404 when no snapshot exists for the platform, and unknown finding ids are
  404.
- 500 with a support reference for a snapshot read failure.

## No-JS contract

Every filter is a GET query value in the URL. A repeated parameter is a 400
page, never a silent population change.

## Idempotency

Safe reads. Export filenames are stable per platform.

## Current deployment

Snapshots come from the runner's account snapshot files; a deployment with no
snapshot for a platform answers the documented 404 form state.
