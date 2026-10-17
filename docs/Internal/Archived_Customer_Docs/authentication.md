# API Authentication

All API requests to the ApexMediation Platform require authentication using JWT (JSON Web Tokens).

## Authentication Methods

We support two authentication methods:

1. **User Authentication** - For dashboard and user-specific operations
2. **Server Authentication** - For server-to-server API calls

---

## User Authentication (Dashboard)

### 1. Register a New Account

**Endpoint**: `POST /api/v1/auth/register`

**Request Body**:
```json
{
  "email": "developer@example.com",
  "password": "<example-password>",
  "firstName": "John",
  "lastName": "Doe",
  "company": "Example Games Inc"
}
```

**Response** (201 Created):
```json
{
  "user": {
    "id": "usr_abc123",
    "email": "developer@example.com",
    "firstName": "John",
    "lastName": "Doe",
    "company": "Example Games Inc",
    "role": "customer",
    "createdAt": "2025-01-15T10:30:00Z"
  },
  "accessToken": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expiresIn": 900
}
```

**Notes:**
- Access token expires in 15 minutes
- Refresh token set as HTTP-only cookie (7 days validity)
- Password requirements: min 8 characters, 1 uppercase, 1 lowercase, 1 number, 1 special character

### 2. Login

**Endpoint**: `POST /api/v1/auth/login`

**Request Body**:
```json
{
  "email": "developer@example.com",
  "password": "<example-password>"
}
```

**Response** (200 OK):
```json
{
  "user": {
    "id": "usr_abc123",
    "email": "developer@example.com",
    "firstName": "John",
    "lastName": "Doe",
    "company": "Example Games Inc",
    "role": "customer"
  },
  "accessToken": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expiresIn": 900
}
```

**Error Responses**:
- `401 Unauthorized` - Invalid credentials
- `429 Too Many Requests` - Rate limit exceeded (5 attempts per 15 minutes)

### 3. Refresh Access Token

**Endpoint**: `POST /api/v1/auth/refresh-token`

**Request**: No body required (refresh token sent as HTTP-only cookie)

**Response** (200 OK):
```json
{
  "accessToken": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expiresIn": 900
}
```

**Notes:**
- Call this endpoint when you receive a `401` error
- Refresh token is automatically rotated for security
- If refresh token is expired, user must login again

### 4. Logout

**Endpoint**: `POST /api/v1/auth/logout`

**Headers**:
```
Authorization: Bearer YOUR_ACCESS_TOKEN
```

**Response** (200 OK):
```json
{
  "message": "Logged out successfully"
}
```

---

## Server Authentication (API Keys)

For server-to-server integrations, use API keys instead of user tokens.

### 1. Generate API Key

1. Log in to the dashboard
2. Go to **Settings → API Keys**
3. Click "Generate New API Key"
4. Name your key (e.g., "Production Server")
5. Copy the key immediately (it won't be shown again)

**API Key Format**: `sk_live_abc123def456ghi789` or `sk_test_abc123def456ghi789`

**Types:**
- `sk_test_*` - Test mode (sandbox data, no real money)
- `sk_live_*` - Production mode (real traffic, real payments)

### 2. Using API Keys

Include the API key in the `Authorization` header:

```bash
curl https://api.apexmediation.ee/v1/campaigns \
  -H "Authorization: Bearer sk_live_abc123def456ghi789"
```

**JavaScript Example**:
```javascript
const response = await fetch('https://api.apexmediation.ee/v1/campaigns', {
  headers: {
    'Authorization': 'Bearer sk_live_abc123def456ghi789',
    'Content-Type': 'application/json'
  }
});
```

**Python Example**:
```python
import requests

headers = {
    'Authorization': 'Bearer sk_live_abc123def456ghi789',
    'Content-Type': 'application/json'
}

response = requests.get('https://api.apexmediation.ee/v1/campaigns', headers=headers)
```

### 3. Revoking API Keys

To revoke an API key:
1. Go to **Settings → API Keys**
2. Find the key you want to revoke
3. Click "Revoke"
4. Confirm the action

**Note**: Revoked keys stop working immediately. Update your servers before revoking.

---

## Making Authenticated Requests

### Using Access Token (User Auth)

**Example: Get User Profile**

```bash
curl https://api.apexmediation.ee/v1/users/me \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
```

**Response** (200 OK):
```json
{
  "id": "usr_abc123",
  "email": "developer@example.com",
  "firstName": "John",
  "lastName": "Doe",
  "company": "Example Games Inc",
  "role": "customer",
  "createdAt": "2025-01-15T10:30:00Z",
  "apps": [
    {
      "id": "app_xyz789",
      "name": "My Awesome Game",
      "platform": "unity",
      "status": "active"
    }
  ]
}
```

### Using API Key (Server Auth)

**Example: Create a Campaign**

```bash
curl -X POST https://api.apexmediation.ee/v1/campaigns \
  -H "Authorization: Bearer sk_live_abc123def456ghi789" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Winter Sale Campaign",
    "budget": 5000.00,
    "currency": "EUR",
    "startDate": "2025-02-01",
    "endDate": "2025-02-28",
    "targetCountries": ["EE", "LV", "LT"]
  }'
```

**Response** (201 Created):
```json
{
  "id": "cmp_def456",
  "name": "Winter Sale Campaign",
  "budget": 5000.00,
  "currency": "EUR",
  "spent": 0.00,
  "status": "scheduled",
  "startDate": "2025-02-01T00:00:00Z",
  "endDate": "2025-02-28T23:59:59Z",
  "targetCountries": ["EE", "LV", "LT"],
  "createdAt": "2025-01-15T14:20:00Z"
}
```

---

## Token Storage Best Practices

### ✅ DO

1. **Store access tokens in memory** (variables, Redux store)
2. **Store refresh tokens in HTTP-only cookies** (automatic, secure)
3. **Use HTTPS** for all API requests
4. **Implement automatic token refresh** when receiving 401 errors
5. **Clear tokens on logout**

### ❌ DON'T

1. **Never store tokens in localStorage** (vulnerable to XSS attacks)
2. **Never store tokens in sessionStorage** (same vulnerability as localStorage)
3. **Never log tokens** to console or error tracking services
4. **Never send tokens in URL parameters** (visible in logs)
5. **Never share API keys** via email, Slack, or messages

---

## Handling Token Expiration

### Automatic Refresh Flow

```javascript
// Example: Axios interceptor for automatic token refresh

import axios from 'axios';

const api = axios.create({
  baseURL: 'https://api.apexmediation.ee/v1',
  withCredentials: true // Send cookies for refresh token
});

// Request interceptor: Add access token to all requests
api.interceptors.request.use((config) => {
  const token = localStorage.getItem('accessToken');
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// Response interceptor: Handle 401 and refresh token
api.interceptors.response.use(
  (response) => response,
  async (error) => {
    const originalRequest = error.config;
    
    // If 401 and not already retried, try to refresh
    if (error.response?.status === 401 && !originalRequest._retry) {
      originalRequest._retry = true;
      
      try {
        const { data } = await axios.post(
          'https://api.apexmediation.ee/v1/auth/refresh-token',
          {},
          { withCredentials: true }
        );
        
        // Store new access token
        localStorage.setItem('accessToken', data.accessToken);
        
        // Retry original request with new token
        originalRequest.headers.Authorization = `Bearer ${data.accessToken}`;
        return api(originalRequest);
      } catch (refreshError) {
        // Refresh failed, redirect to login
        localStorage.removeItem('accessToken');
        window.location.href = '/signin';
        return Promise.reject(refreshError);
      }
    }
    
    return Promise.reject(error);
  }
);

export default api;
```

---

## Security Considerations

### API Key Rotation

**Best Practice**: Rotate API keys every 90 days.

**Steps**:
1. Generate a new API key
2. Update your servers to use the new key
3. Monitor for errors for 24 hours
4. Revoke the old API key

### IP Allowlisting

Restrict API keys to specific IP addresses:
1. Go to **Settings → API Keys**
2. Click on an API key
3. Add allowed IP addresses (e.g., `203.0.113.1, 203.0.113.2`)
4. Save changes

**Note**: Requests from non-whitelisted IPs will receive `403 Forbidden`.

### Webhook Verification

When receiving webhooks from our platform, verify the signature:

```javascript
const crypto = require('crypto');

function verifyWebhookSignature(payload, signature, secret) {
  const expectedSignature = crypto
    .createHmac('sha256', secret)
    .update(payload)
    .digest('hex');
  
  return crypto.timingSafeEqual(
    Buffer.from(signature),
    Buffer.from(expectedSignature)
  );
}

// Express middleware
app.post('/webhooks/apexmediation', (req, res) => {
  const signature = req.headers['x-apexmediation-signature'];
  const isValid = verifyWebhookSignature(
    JSON.stringify(req.body),
    signature,
    process.env.WEBHOOK_SECRET
  );
  
  if (!isValid) {
    return res.status(403).json({ error: 'Invalid signature' });
  }
  
  // Process webhook...
  res.json({ received: true });
});
```

---

## Rate Limits

All authentication endpoints are rate-limited to prevent brute-force attacks:

| Endpoint | Limit | Window |
|----------|-------|--------|
| `POST /auth/login` | 5 requests | 15 minutes |
| `POST /auth/register` | 3 requests | 1 hour |
| `POST /auth/refresh-token` | 10 requests | 1 minute |

**Headers in Response**:
```
X-RateLimit-Limit: 5
X-RateLimit-Remaining: 3
X-RateLimit-Reset: 1642248000
Retry-After: 600
```

**429 Too Many Requests Response**:
```json
{
  "error": "Rate limit exceeded",
  "message": "Too many login attempts. Please try again in 10 minutes.",
  "retryAfter": 600
}
```

---

## Testing Authentication

### Test Credentials (Sandbox)

For development, use these test credentials:

**Email**: `test@apexmediation.ee`  
**Password**: `TestPassword123!`  
**API Key**: `sk_test_1234567890abcdef`

**Notes:**
- Test environment uses separate database
- No real money transactions
- Test ads only (no real advertiser demand)

### Authentication Testing Tools

**Postman Collection**: [Download](https://api.apexmediation.ee/docs/postman-collection.json)

**cURL Examples**: See [API Reference](/docs/api-reference/rest-api)

---

## Troubleshooting

### Common Errors

**401 Unauthorized - "Invalid token"**
- Token has expired → Use refresh token endpoint
- Token is malformed → Check if you're sending the full token
- Token is for wrong environment (test vs live) → Check token prefix

**401 Unauthorized - "Invalid credentials"**
- Email or password is incorrect
- Account is locked (too many failed attempts) → Wait 15 minutes
- Account is not verified → Check email for verification link

**403 Forbidden - "Insufficient permissions"**
- Your account role doesn't have access to this resource
- API key is revoked → Generate a new key
- IP is not whitelisted → Add your IP to allowed list

**429 Too Many Requests**
- Wait for the time specified in `Retry-After` header
- Implement exponential backoff in your code
- Consider caching responses to reduce API calls

---

## Support

- **📧 Email**: security@apexmediation.ee
- **📚 API Reference**: [docs.apexmediation.ee/api](https://docs.apexmediation.ee/api)
- **🐛 Report Security Issue**: [security@apexmediation.ee](mailto:security@apexmediation.ee)
- **⏱️ Response Time**: < 2 hours for security issues, < 4 hours for other issues

---

**Last Updated**: January 2025  
**API Version**: v1  
**Status**: Stable
