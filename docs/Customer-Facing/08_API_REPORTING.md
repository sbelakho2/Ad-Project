# Reporting API

## Overview

The Reporting API allows you to programmatically retrieve performance data for your apps and ad units.

**Base URL**: `https://api.apexmediation.ee/api/v1/reporting`

## Authentication

All requests must include your API Key in the `Authorization` header.

```http
Authorization: Bearer YOUR_API_KEY
```

## Endpoints

### 1. Revenue Overview

`GET /overview`

Get high-level revenue statistics for a date range.

**Parameters:**

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `startDate` | String | Yes | ISO8601 date (e.g., `2023-10-01`). |
| `endDate` | String | Yes | ISO8601 date (e.g., `2023-10-07`). |

**Response:**

```json
{
  "revenue": 1250.50,
  "impressions": 50000,
  "ecpm": 25.01,
  "fillRate": 0.92
}
```

### 2. Time Series Data

`GET /timeseries`

Get performance data grouped by time for charting.

**Parameters:**

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `startDate` | String | Yes | ISO8601 date. |
| `endDate` | String | Yes | ISO8601 date. |
| `granularity` | String | No | `hour` or `day` (default: `day`). |

### 3. Adapter Performance

`GET /adapters`

Breakdown of performance by ad network adapter.

**Parameters:**

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `startDate` | String | Yes | ISO8601 date. |
| `endDate` | String | Yes | ISO8601 date. |

### 4. Country Breakdown

`GET /countries`

Revenue and impressions by country.

**Parameters:**

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `startDate` | String | Yes | ISO8601 date. |
| `endDate` | String | Yes | ISO8601 date. |
| `limit` | Integer | No | Max results (default: 10). |

### 5. Top Apps

`GET /top-apps`

Performance by application.

**Parameters:**

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `startDate` | String | Yes | ISO8601 date. |
| `endDate` | String | Yes | ISO8601 date. |
| `limit` | Integer | No | Max results (default: 10). |

### 6. Real-time Stats

`GET /realtime`

Get statistics for the last hour. Useful for monitoring live issues.

**Parameters:** None.

## Rate Limits

*   **100 requests per minute** per API Key.
*   **10,000 requests per day** per API Key.

If you exceed these limits, you will receive a `429 Too Many Requests` response.
