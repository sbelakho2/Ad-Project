# Website-to-Dashboard Authentication Integration

This document explains how the marketing website seamlessly integrates with the customer dashboard (console).

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────┐
│                      CUSTOMER JOURNEY                                 │
└──────────────────────────────────────────────────────────────────────┘

  website.apexmediation.ee                     console.apexmediation.ee
  (Marketing & Public)                   (Authenticated Dashboard)
          │                                       │
          │ 1. User clicks "Sign In"             │
          ├──────────────────────────────────────>│
          │                                       │
          │ 2. User enters credentials            │
          │    POST /api/v1/auth/login           │
          │<──────────────────────────────────────┤
          │                                       │
          │ 3. Backend returns JWT + refresh token│
          │    (HTTP-only cookie)                 │
          │                                       │
          │ 4. Redirect to dashboard              │
          ├──────────────────────────────────────>│
          │                                       │
          │                                5. User sees dashboard
          │                                   (JWT in localStorage)
          │                                       │
```

## Implementation

### 1. Shared Authentication Backend

Both the website and dashboard use the **same backend API** for authentication:

**Backend URL**: `https://api.apexmediation.ee/v1`

**Endpoints**:
- `POST /api/v1/auth/register` - Create account
- `POST /api/v1/auth/login` - Login
- `POST /api/v1/auth/refresh-token` - Refresh expired token
- `POST /api/v1/auth/logout` - Logout
- `GET /api/v1/users/me` - Get current user

### 2. Website Sign In Flow

**File**: `website/src/app/(marketing)/signin/page.tsx`

```tsx
'use client';

import { useState } from 'react';
import { useRouter } from 'next/navigation';
import api from '@/lib/api';
import { useAuthStore } from '@/store/auth';

export default function SignInPage() {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  const router = useRouter();
  const { setUser, setToken } = useAuthStore();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    setLoading(true);

    try {
      const { data } = await api.post('/auth/login', { email, password });

      // Store user data and token
      setUser(data.user);
      setToken(data.accessToken);
      localStorage.setItem('accessToken', data.accessToken);

      // Redirect to dashboard (console subdomain)
      window.location.href = 'https://console.apexmediation.ee/dashboard';

    } catch (err: any) {
      setError(err.response?.data?.message || 'Login failed');
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen bg-primary-blue flex items-center justify-center">
      <div className="card card-yellow w-full max-w-md">
        <h1 className="text-3xl font-bold uppercase mb-6">Sign In</h1>

        {error && (
          <div className="bg-accent-red text-white p-4 mb-4">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit}>
          <div className="mb-4">
            <label className="label">Email</label>
            <input
              type="email"
              className="input"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
            />
          </div>

          <div className="mb-6">
            <label className="label">Password</label>
            <input
              type="password"
              className="input"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
            />
          </div>

          <button
            type="submit"
            className="btn btn-primary w-full"
            disabled={loading}
          >
            {loading ? 'Signing In...' : 'Sign In →'}
          </button>
        </form>

        <div className="mt-6 text-center">
          <a href="/forgot-password" className="text-sm">
            Forgot Password?
          </a>
          <p className="mt-2 text-sm">
            Don't have an account?{' '}
            <a href="/signup" className="font-bold">
              Sign Up
            </a>
          </p>
        </div>
      </div>
    </div>
  );
}
```

### 3. Dashboard Authentication Check

**File**: `console/src/middleware.ts`

```typescript
import { NextResponse } from 'next/server';
import type { NextRequest } from 'next/server';
import { jwtDecode } from 'jwt-decode';

export function middleware(request: NextRequest) {
  const token = request.cookies.get('accessToken')?.value
    || request.headers.get('authorization')?.replace('Bearer ', '')
    || (typeof window !== 'undefined' ? localStorage.getItem('accessToken') : null);

  const isAuthPage = request.nextUrl.pathname.startsWith('/login');
  const isPublicPage = request.nextUrl.pathname === '/' || isAuthPage;

  // Redirect to login if accessing protected page without token
  if (!token && !isPublicPage) {
    return NextResponse.redirect(new URL('/login', request.url));
  }

  // Check if token is expired
  if (token) {
    try {
      const decoded: any = jwtDecode(token);
      const now = Date.now() / 1000;

      if (decoded.exp < now) {
        // Token expired, redirect to login
        return NextResponse.redirect(new URL('/login?expired=true', request.url));
      }
    } catch (error) {
      // Invalid token, redirect to login
      return NextResponse.redirect(new URL('/login', request.url));
    }
  }

  // Redirect to dashboard if accessing login page with valid token
  if (token && isAuthPage) {
    return NextResponse.redirect(new URL('/dashboard', request.url));
  }

  return NextResponse.next();
}

export const config = {
  matcher: [
    /*
     * Match all request paths except:
     * - api (API routes)
     * - _next/static (static files)
     * - _next/image (image optimization files)
     * - favicon.ico (favicon file)
     */
    '/((?!api|_next/static|_next/image|favicon.ico).*)',
  ],
};
```

### 4. Shared API Client

Both website and console use the same API client configuration for consistency.

**File**: `console/src/lib/api.ts` (mirrors `website/src/lib/api.ts`)

```typescript
import axios from 'axios';

const api = axios.create({
  baseURL: process.env.NEXT_PUBLIC_API_URL || 'https://api.apexmediation.ee/v1',
  timeout: 30000,
  withCredentials: true, // Send cookies for refresh tokens
  headers: {
    'Content-Type': 'application/json',
  },
});

// Request interceptor: Add JWT to headers
api.interceptors.request.use((config) => {
  const token = localStorage.getItem('accessToken');
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// Response interceptor: Handle 401, refresh token
api.interceptors.response.use(
  (response) => response,
  async (error) => {
    const originalRequest = error.config;

    if (error.response?.status === 401 && !originalRequest._retry) {
      originalRequest._retry = true;

      try {
        const { data } = await axios.post(
          `${process.env.NEXT_PUBLIC_API_URL}/auth/refresh-token`,
          {},
          { withCredentials: true }
        );

        localStorage.setItem('accessToken', data.accessToken);
        originalRequest.headers.Authorization = `Bearer ${data.accessToken}`;

        return api(originalRequest);
      } catch (refreshError) {
        // Refresh failed, redirect to login
        localStorage.removeItem('accessToken');
        window.location.href = 'https://website.apexmediation.ee/signin';
        return Promise.reject(refreshError);
      }
    }

    return Promise.reject(error);
  }
);

export default api;
```

### 5. Cross-Domain Token Sharing

Since website and console are on different subdomains, we use a combination of:

1. **localStorage** for access tokens (short-lived, 15 minutes)
2. **HTTP-only cookies** for refresh tokens (long-lived, 7 days, domain: `.apexmediation.ee`)

**Backend Cookie Configuration**:

```typescript
// backend/src/controllers/AuthController.ts

res.cookie('refreshToken', refreshToken, {
  httpOnly: true,
  secure: process.env.NODE_ENV === 'production', // HTTPS only in production
  sameSite: 'lax',
  domain: '.apexmediation.ee', // Works for all subdomains
  maxAge: 7 * 24 * 60 * 60 * 1000, // 7 days
  path: '/',
});
```

### 6. Seamless Navigation Between Sites

**Website Header** (logged in state):

```tsx
// website/src/components/marketing/Header.tsx

import { useAuthStore } from '@/store/auth';

export default function Header() {
  const { user, isAuthenticated } = useAuthStore();

  return (
    <header>
      <nav>
        {/* ...other nav items... */}

        {isAuthenticated ? (
          <>
            <a href="https://console.apexmediation.ee/dashboard" className="btn btn-secondary">
              Dashboard →
            </a>
            <span className="text-sunshine-yellow">
              Hello, {user?.firstName}!
            </span>
          </>
        ) : (
          <>
            <a href="/signin" className="btn btn-outline">
              Sign In
            </a>
            <a href="/signup" className="btn btn-primary">
              Get Started →
            </a>
          </>
        )}
      </nav>
    </header>
  );
}
```

**Console Header** (with back to website link):

```tsx
// console/src/components/Navigation.tsx

export default function Navigation() {
  return (
    <nav className="bg-primary-blue border-b border-sunshine-yellow">
      <div className="max-w-7xl mx-auto px-4">
        <div className="flex justify-between h-16">
          <div className="flex">
            <a href="https://website.apexmediation.ee" className="flex items-center text-sunshine-yellow font-bold uppercase">
              ← AD ENGINE
            </a>

            {/* Dashboard nav items */}
            <a href="/dashboard" className="nav-link">Dashboard</a>
            <a href="/analytics" className="nav-link">Analytics</a>
            <a href="/campaigns" className="nav-link">Campaigns</a>
            <a href="/payouts" className="nav-link">Payouts</a>
          </div>

          <div className="flex items-center">
            <UserMenu />
          </div>
        </div>
      </div>
    </nav>
  );
}
```

### 7. Environment Variables

**Website** (`.env.local`):

```bash
NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/v1
NEXT_PUBLIC_CONSOLE_URL=https://console.apexmediation.ee
NEXT_PUBLIC_SITE_URL=https://website.apexmediation.ee
```

**Console** (`.env.local`):

```bash
NEXT_PUBLIC_API_URL=https://api.apexmediation.ee/v1
NEXT_PUBLIC_WEBSITE_URL=https://website.apexmediation.ee
```

**Backend** (`.env`):

```bash
FRONTEND_URL=https://website.apexmediation.ee
CONSOLE_URL=https://console.apexmediation.ee
CORS_ORIGIN=https://website.apexmediation.ee,https://console.apexmediation.ee
COOKIE_DOMAIN=.apexmediation.ee
```

### 8. CORS Configuration

**Backend** must allow requests from both domains:

```typescript
// backend/src/index.ts

import cors from 'cors';

const allowedOrigins = [
  'https://website.apexmediation.ee',
  'https://console.apexmediation.ee',
  'http://localhost:3000', // Website dev
  'http://localhost:3001', // Console dev
];

app.use(cors({
  origin: (origin, callback) => {
    if (!origin || allowedOrigins.includes(origin)) {
      callback(null, true);
    } else {
      callback(new Error('Not allowed by CORS'));
    }
  },
  credentials: true, // Allow cookies
}));
```

### 9. Logout Flow

Logout from either website or console logs out from both:

```typescript
// Shared logout function

export async function logout() {
  try {
    // Call backend logout endpoint (clears refresh token cookie)
    await api.post('/auth/logout');
  } catch (error) {
    console.error('Logout error:', error);
  } finally {
    // Clear local storage
    localStorage.removeItem('accessToken');
    localStorage.removeItem('user');

    // Redirect to website login
    window.location.href = 'https://website.apexmediation.ee/signin';
  }
}
```

### 10. Testing the Integration

**Test Checklist**:

- [ ] User can sign up on website → Automatically logged in
- [ ] User can sign in on website → Redirected to console dashboard
- [ ] User can navigate to console → Session persists
- [ ] Token expires → Automatic refresh without re-login
- [ ] Refresh token expires → Redirected to login
- [ ] User logs out from console → Cannot access dashboard without re-login
- [ ] User clicks "Dashboard" from website → Taken directly to dashboard (if logged in)
- [ ] CORS allows requests from both domains
- [ ] Cookies set with correct domain (`.apexmediation.ee`)

**Testing Commands**:

```bash
# Start website
cd website && npm run dev # Port 3000

# Start console
cd console && npm run dev # Port 3001

# Start backend
cd backend && npm run dev # Port 4000

# Test login
curl -X POST http://localhost:4000/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"password123"}' \
  -c cookies.txt

# Test protected route with cookie
curl http://localhost:4000/api/v1/users/me \
  -H "Authorization: Bearer YOUR_ACCESS_TOKEN" \
  -b cookies.txt
```

---

## Security Considerations

### 1. Token Storage

**Access Token** (localStorage):
- ✅ Short-lived (15 minutes)
- ✅ Can be stolen via XSS
- ✅ **Mitigation**: Use Content Security Policy (CSP)

**Refresh Token** (HTTP-only cookie):
- ✅ Long-lived (7 days)
- ✅ Cannot be accessed by JavaScript
- ✅ Protected from XSS
- ⚠️ Vulnerable to CSRF
- ✅ **Mitigation**: SameSite=Lax + CSRF tokens

### 2. HTTPS Only

**All domains must use HTTPS** in production:
- `https://website.apexmediation.ee`
- `https://console.apexmediation.ee`
- `https://api.apexmediation.ee`

**Certificate**: Use Cloudflare or Let's Encrypt

### 3. Content Security Policy

```typescript
// website/next.config.js

const securityHeaders = [
  {
    key: 'Content-Security-Policy',
    value: `
      default-src 'self';
      script-src 'self' 'unsafe-eval' 'unsafe-inline' https://www.googletagmanager.com;
      style-src 'self' 'unsafe-inline';
      img-src 'self' data: https:;
      font-src 'self' data:;
      connect-src 'self' https://api.apexmediation.ee;
      frame-ancestors 'none';
    `.replace(/\s{2,}/g, ' ').trim()
  },
];

module.exports = {
  async headers() {
    return [
      {
        source: '/:path*',
        headers: securityHeaders,
      },
    ];
  },
};
```

### 4. Rate Limiting

**Backend** implements **tiered rate limiting** based on customer subscription tier. Limits scale from Starter to Enterprise.

**Rate Limit Configuration** (`backend/src/config/rateLimits.ts`):

| Endpoint Category | Starter | Growth | Scale | Enterprise |
|-------------------|---------|--------|-------|------------|
| **Auth** (login, register) | 20/min | 50/min | 100/min | 200/min |
| **Global** (all API) | 1,000/min | 5,000/min | 20,000/min | 100,000/min |
| **Dashboard** | 300/min | 1,000/min | 3,000/min | 10,000/min |
| **Analytics** (SDK) | 5,000/min | 25,000/min | 100,000/min | 500,000/min |

**Security-Sensitive Endpoints** (stricter limits):
- Login: 5 attempts/minute per IP
- Register: 3 attempts/5 minutes per IP
- Password Reset: 3 attempts/5 minutes per IP
- 2FA Verification: 5 attempts/minute per IP

```typescript
// backend/src/middleware/tieredRateLimiter.ts

import { createTieredRateLimiter, authTieredRateLimiter } from './middleware/tieredRateLimiter';

// Auth routes use stricter tier-aware limiter
app.use('/api/v1/auth', authTieredRateLimiter);

// Global rate limiter applied to all routes
app.use('/api/v1/', limiter);
```

**Rate Limit Headers** (standard):
- `X-RateLimit-Limit`: Maximum requests allowed
- `X-RateLimit-Remaining`: Requests remaining in window
- `X-RateLimit-Reset`: Unix timestamp when window resets
- `Retry-After`: Seconds until retry allowed (on 429)

**Exempt Routes** (no rate limiting):
- `/health`, `/ready`, `/metrics` (monitoring)
- `/api/v1/status.json`, `/api/v1/slo.json` (status page)
- `/api/v1/public/*` (public cacheable endpoints)

---

## Deployment

### DNS Configuration

```
website.apexmediation.ee    A    104.21.x.x (Cloudflare)
console.apexmediation.ee    A    104.21.x.x (Cloudflare)
api.apexmediation.ee        A    Your-Backend-IP
```

### Vercel Configuration

**Website**:
- Project: `apexmediation-website`
- Domain: `website.apexmediation.ee`
- Environment Variables: `NEXT_PUBLIC_API_URL`, `NEXT_PUBLIC_CONSOLE_URL`

**Console**:
- Project: `apexmediation-console`
- Domain: `console.apexmediation.ee`
- Environment Variables: `NEXT_PUBLIC_API_URL`, `NEXT_PUBLIC_WEBSITE_URL`

---

## Troubleshooting

### "CORS policy: No 'Access-Control-Allow-Origin' header"

**Solution**: Check backend CORS configuration allows both website and console origins.

### "Invalid token" after refresh

**Solution**: Check refresh token cookie domain is set to `.apexmediation.ee` (with leading dot).

### "Token not found" when navigating from website to console

**Solution**: Ensure `withCredentials: true` in API client configuration.

### Session lost after closing browser

**Solution**: This is expected behavior. Refresh token is session cookie by default. To persist:

```typescript
res.cookie('refreshToken', refreshToken, {
  httpOnly: true,
  secure: true,
  sameSite: 'lax',
  domain: '.apexmediation.ee',
  maxAge: 7 * 24 * 60 * 60 * 1000, // 7 days
  path: '/',
});
```

---

## Status

**Implementation Status**:
- [x] Shared backend authentication
- [x] Website sign in/up pages
- [x] Console authentication middleware
- [x] Cross-domain cookie configuration
- [x] API client with token refresh
- [x] Seamless navigation links
- [x] Logout flow
- [ ] E2E tests for auth flow
- [ ] Security audit

**Next Steps**:
1. Deploy to staging environment
2. Test full auth flow end-to-end
3. Security audit
4. Performance testing (token refresh under load)
5. Deploy to production

---

**Last Updated**: January 2025
**Owner**: Full-Stack Team
**Status**: Ready for Testing
