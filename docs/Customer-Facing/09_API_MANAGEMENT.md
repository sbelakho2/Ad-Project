# Management API

## Overview

The Management API allows you to programmatically configure your placements and waterfalls.

**Base URL**: `https://api.apexmediation.ee/api/v1`

## Authentication

All requests must include your API Key in the `Authorization` header.

```http
Authorization: Bearer YOUR_API_KEY
```

## Resources

### Placements

#### List Placements

`GET /placements`

Returns a paginated list of placements.

**Parameters:**

| Parameter | Type | Description |
| :--- | :--- | :--- |
| `page` | Integer | Page number (1-based). |
| `pageSize` | Integer | Items per page (default 50, max 200). |
| `format` | String | Filter by format (`banner`, `interstitial`, `rewarded`, `native`). |

#### Create Placement

`POST /placements`

**Request Body:**

```json
{
  "name": "Level Complete Interstitial",
  "appId": "app_12345",
  "format": "interstitial",
  "config": {
    "waterfall": [
      {
        "network": "admob",
        "priority": 1,
        "params": { "adUnitId": "ca-app-pub-xxx" }
      }
    ]
  }
}
```

#### Get Placement

`GET /placements/{id}`

Returns details for a specific placement.

#### Update Placement

`PUT /placements/{id}`

Updates a placement's configuration.

**Request Body:**

```json
{
  "name": "New Name",
  "config": {
    // Full replacement of config object
  }
}
```

## Error Handling

*   `400 Bad Request`: Invalid parameters or JSON body.
*   `401 Unauthorized`: Invalid or missing API Key.
*   `403 Forbidden`: Insufficient permissions.
*   `404 Not Found`: Resource does not exist.
