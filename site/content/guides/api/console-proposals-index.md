+++
title = "Console proposal index | Apex API reference"
description = "List every proposed change with its recorded decision state."
template = "api.html"
weight = 13

[extra]
h1 = "Console proposal index"
og_image = "og/guides.png"
+++

## Purpose

List every proposal in the workspace store with its setting, creation time
and decision state, so one page answers which approvals are waiting.

## Method and path

`GET /console/proposals` renders the index; each row links to
`GET /console/proposals/{id}`.

## Authorization and CSRF

A live change-log session is required; without one the request is
303-redirected to the change-log access panel.

## Fields

No query fields. The table shows:

- Proposal: the stable proposal id.
- Setting: the proposed change with its scope.
- Created: the creation time in UTC.
- State: approval required, approved, declined or expired.
- Action: a link to the proposal page.

Proposal files that cannot be read are counted in the table note rather
than dropped silently.

## States

- Populated: every readable proposal, newest first, bounded per page.
- Empty: "No proposal is open." with a link to the experiment list.
- Skipped files: the count note names how many files could not be read.

## Failure states

- 303 to the access panel without a live session.
- 500 when the proposal directory cannot be read.
- 503 when the proposal store or workspace publisher is not configured.

## No-JS contract

A plain GET page; every action is a link to a server-rendered page.

## Idempotency

Read-only: the index records nothing.

## Current deployment

Proposals are read from the workspace proposal store; a deployment without
one answers the documented 503 page.
