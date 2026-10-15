+++
title = "GET /healthz | Apex API reference"
description = "200 status JSON. Liveness endpoint."
template = "api.html"
weight = 1

[extra]
h1 = "GET /healthz"
og_image = "og/guides.png"
+++

## Purpose

200 status JSON. Liveness endpoint.

## Method and path

`GET /healthz`

## Authorization

None.

## Request fields

None.

## Sample response

```json
{"status": "ok"}
```

## Failure states

A request with any other method on this path is refused with 405.

## Idempotency

A safe read with no side effects; repeating it changes nothing.

## Current deployment

Liveness is not data freshness or write readiness. A 200 answers while the process serves; it does not say that snapshot data is fresh or that writes are available.
