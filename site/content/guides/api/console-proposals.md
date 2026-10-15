+++
title = "Console proposals | Apex API reference"
description = "Review a proposed change and record an approve or decline decision."
template = "api.html"
weight = 14

[extra]
h1 = "Console proposals"
og_image = "og/guides.png"
+++

## Purpose

Review one proposed change with its evidence and record a decision: approve
it for the runner or decline it.

## Method and path

- `GET /console/proposals/{id}` renders the proposal, its recorded decision
  if any, and its expiry state.
- `POST /console/proposals/{id}/decision` records the decision and answers
  303 back to the proposal page.

## Authorization and CSRF

A live change-log session and the hidden `csrf_token` field are required;
without a session the request is 303-redirected to the change-log access
panel, and a wrong token is 400.

## Request fields

`POST /console/proposals/{id}/decision` form fields:

- `decision`: `approve` or `decline`.
- `idempotency_key`: the stable key for this proposal, decision and change
  hash. The page embeds it; a request whose key does not match the expected
  key is 400, so a decision cannot be replayed onto a changed proposal.

## States

- Open: the proposal with its before and after values and the evidence.
- Expired: the page states the proposal is expired; a decision POST
  re-renders the expired state without recording.
- Decided: the recorded decision with its time and actor.
- Conflict: a different decision for the same proposal is 409 with the
  refusal that the recorded decision stands.

## Failure states

- 400 for a missing or wrong CSRF token, an unknown decision value or a
  mismatched idempotency key.
- 303 to the access panel without a live session.
- 404 for an unknown proposal id.
- 409 when the proposal was already decided differently.
- 503 when the proposal store or workspace publisher is not configured.

## No-JS contract

A form POST plus a 303 redirect; the decided state lives on the stable
proposal page.

## Idempotency

A retry with the same decision and idempotency key returns the original
outcome (303) instead of recording twice.

## Current deployment

Proposals are read from the workspace proposal store; a deployment without
one answers the documented 503 page.
