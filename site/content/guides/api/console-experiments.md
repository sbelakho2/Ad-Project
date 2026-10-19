+++
title = "Console experiments | Apex API reference"
description = "List and inspect tests with GET-link filters and tabs; incident recovery state."
template = "api.html"
weight = 15

[extra]
h1 = "Console experiments"
og_image = "og/guides.png"
+++

## Purpose

The test portfolio: every projected experiment with its state, filters kept
in the URL, one detail page with tabs, and the recovery state for a test that
hit an incident.

## Method and path

- `GET /console/experiments?filter={filter}` lists tests. Filters:
  `all` (default), `final` (finalized data only) and `30d` (last thirty
  days).
- `GET /console/experiments/{id}` renders one test; the visible tab is a GET
  value: `summary` (default), `inputs`, `guardrails`, `history`.
- `GET /console/experiments/{id}/incident` renders the recovery state.

## Authorization

A live change-log session is required; without one the request is `303 See
Other` to `/console/ledger`. The pages read the verified change log and only
the alerts whose experiment id appears in this publisher's records;
they mutate nothing.

## Request fields

Filters and tabs travel as GET query values, so every view is linkable. An
unknown filter is a 400 page naming the allowed set; an unknown tab is a 400
page naming the allowed set.

## States

- List: per-test state chips, platform, dates and outcome; the filter and the
  as-of time are visible; count shown.
- Detail: the same state plus the selected tab content (summary KPIs,
  inputs, the guardrails that were armed, or the history timeline).
- Incident: what happened, what the guardrail did, and the current recovery
  step.

## Failure states

- 400 for an unknown filter or tab.
- 404 when the change log does not exist for the workspace, or when the
  experiment id is unknown, or when no incident is recorded for the id.
- 422 when the change log does not verify, with the failing line.
- 500 with a support reference for a read failure.
- 503 when the workspace publisher is not configured.

## No-JS contract

Tabs and filters are links, not script state; the browser back button returns
to the previous view.

## Idempotency

Safe reads.

## Current deployment

Experiments are projected from verified change-log records plus the alert
log. A deployment without either shows the honest empty or unavailable
state.
