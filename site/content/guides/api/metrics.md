+++
title = "GET /metrics | Apex API reference"
description = "Prometheus text: build, status counters, in-flight and uptime."
template = "api.html"
weight = 2

[extra]
h1 = "GET /metrics"
og_image = "og/guides.png"
+++

## Purpose

Prometheus text: build, status counters, in-flight and uptime.

## Method and path

`GET /metrics`

## Authorization

None.

## Request fields

None.

## Sample response

```text
# HELP apex_build_info Build info as labels; value is always 1.
# TYPE apex_build_info gauge
apex_build_info{version="0.1.0"} 1
# HELP apex_api_requests_total Total HTTP requests by status class.
# TYPE apex_api_requests_total counter
apex_api_requests_total{status="2xx"} 3
apex_api_requests_total{status="4xx"} 0
apex_api_requests_total{status="5xx"} 0
# HELP apex_api_requests_in_flight Requests currently being served.
# TYPE apex_api_requests_in_flight gauge
apex_api_requests_in_flight 0
# HELP apex_api_uptime_seconds Seconds since process start.
# TYPE apex_api_uptime_seconds gauge
apex_api_uptime_seconds 128
```

The content type is `text/plain; version=0.0.4; charset=utf-8`.

## Failure states

A request with any other method on this path is refused with 405.

## Idempotency

A safe read; the counters and the uptime keep moving, but no request state changes.

## Current deployment

Operator infrastructure documentation, not main customer navigation. The counters are process lifetime totals and reset with the process.
