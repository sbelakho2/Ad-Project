# Website-Backend Technical Integration Design

Complete technical architecture for connecting the customer-facing website to the ApexMediation backend, providing full access to all platform features.

## Table of Contents

1. [System Architecture](#system-architecture)
2. [Authentication & Authorization](#authentication--authorization)
3. [Dashboard Features](#dashboard-features)
4. [API Endpoints](#api-endpoints)
5. [Real-Time Features](#real-time-features)
6. [Data Models](#data-models)
7. [Implementation Tasks](#implementation-tasks)

---

## System Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                        Customer Website                       │
│                     (Next.js 14 + TypeScript)                 │
├──────────────────────────────────────────────────────────────┤
│  Pages:                                                       │
│  - / (Homepage)                    - /signup (Registration)   │
│  - /dashboard (Main Console)       - /signin (Login)          │
│  - /dashboard/revenue              - /dashboard/analytics     │
│  - /dashboard/networks             - /dashboard/ab-tests      │
│  - /dashboard/fraud                - /dashboard/apps          │
│  - /dashboard/settings             - /documentation           │
└────────────────┬─────────────────────────────────────────────┘
                 │
                 │ HTTPS / JWT Auth
                 │
┌────────────────▼─────────────────────────────────────────────┐
│                      ApexMediation Backend                         │
│              (Node.js/TypeScript + Go Services)               │
├──────────────────────────────────────────────────────────────┤
│  API Layer (Node.js):                                        │
│  - /api/v1/auth/*          - /api/v1/analytics/*            │
│  - /api/v1/revenue/*       - /api/v1/ab-tests/*             │
│  - /api/v1/fraud/*         - /api/v1/adapters/*             │
│  - /api/v1/apps/*          - /api/v1/placements/*           │
│                                                               │
│  Microservices (Go):                                         │
│  - Auction Service (port 8080)  - Header Bidding             │
│  - Fraud Service (port 8081)    - ML Detection               │
│  - Analytics Service (port 8082) - Postgres Rollups          │
└────────────────┬─────────────────────────────────────────────┘
                 │
                 │
┌────────────────▼─────────────────────────────────────────────┐
│                        Data Layer                             │
├──────────────────────────────────────────────────────────────┤
│  PostgreSQL: User data, apps, configs, experiments           │
│  Postgres analytics replicas: impressions, revenue, rollups  │
│  Redis: Caching, session state, real-time data               │
└──────────────────────────────────────────────────────────────┘
```

---

## Authentication & Authorization

### JWT-Based Authentication

**Flow:**
1. User submits email/password to `/api/v1/auth/login`
2. Backend validates credentials, generates JWT token
3. Token contains: `{ userId, publisherId, email, role, exp }`
4. Token stored in `httpOnly` cookie + returned in response
5. All subsequent requests include token in `Authorization: Bearer <token>`

**Implementation:**

```typescript
// website/src/lib/auth.ts
import { cookies } from 'next/headers';
import { jwtVerify, SignJWT } from 'jose';

const JWT_SECRET = new TextEncoder().encode(
  process.env.JWT_SECRET || 'your-secret-key'
);

export interface User {
  id: string;
  email: string;
  publisherId: string;
  role: 'admin' | 'developer' | 'viewer';
}

export async function signToken(user: User): Promise<string> {
  return new SignJWT({
    userId: user.id,
    publisherId: user.publisherId,
    email: user.email,
    role: user.role,
  })
    .setProtectedHeader({ alg: 'HS256' })
    .setExpirationTime('7d')
    .setIssuedAt()
    .sign(JWT_SECRET);
}

export async function verifyToken(token: string): Promise<User | null> {
  try {
    const { payload } = await jwtVerify(token, JWT_SECRET);
    return {
      id: payload.userId as string,
      email: payload.email as string,
      publisherId: payload.publisherId as string,
      role: payload.role as 'admin' | 'developer' | 'viewer',
    };
  } catch {
    return null;
  }
}

export async function getSession(): Promise<User | null> {
  const cookieStore = cookies();
  const token = cookieStore.get('session')?.value;
  if (!token) return null;
  return verifyToken(token);
}
```

**API Routes:**

```typescript
// website/src/app/api/auth/login/route.ts
import { NextRequest, NextResponse } from 'next/server';
import { signToken } from '@/lib/auth';

export async function POST(request: NextRequest) {
  const { email, password } = await request.json();

  // Call backend auth endpoint
  const response = await fetch(`${process.env.BACKEND_URL}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }),
  });

  if (!response.ok) {
    return NextResponse.json(
      { error: 'Invalid credentials' },
      { status: 401 }
    );
  }

  const { data } = await response.json();
  const user = data.user;

  // Generate JWT for website
  const token = await signToken(user);

  // Set httpOnly cookie
  const res = NextResponse.json({ success: true, user });
  res.cookies.set('session', token, {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'lax',
    maxAge: 60 * 60 * 24 * 7, // 7 days
    path: '/',
  });

  return res;
}
```

```typescript
// website/src/app/api/auth/logout/route.ts
import { NextResponse } from 'next/server';

export async function POST() {
  const res = NextResponse.json({ success: true });
  res.cookies.delete('session');
  return res;
}
```

```typescript
// website/src/app/api/auth/me/route.ts
import { NextResponse } from 'next/server';
import { getSession } from '@/lib/auth';

export async function GET() {
  const user = await getSession();
  if (!user) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }
  return NextResponse.json({ user });
}
```

**Middleware:**

```typescript
// website/src/middleware.ts
import { NextResponse } from 'next/server';
import type { NextRequest } from 'next/server';
import { verifyToken } from '@/lib/auth';

export async function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Public routes
  if (
    pathname === '/' ||
    pathname.startsWith('/signin') ||
    pathname.startsWith('/signup') ||
    pathname.startsWith('/api/auth/login') ||
    pathname.startsWith('/api/auth/signup')
  ) {
    return NextResponse.next();
  }

  // Protected routes
  if (pathname.startsWith('/dashboard') || pathname.startsWith('/api/')) {
    const token = request.cookies.get('session')?.value;
    if (!token) {
      return NextResponse.redirect(new URL('/signin', request.url));
    }

    const user = await verifyToken(token);
    if (!user) {
      return NextResponse.redirect(new URL('/signin', request.url));
    }

    // Add user to headers for API routes
    const requestHeaders = new Headers(request.headers);
    requestHeaders.set('x-user-id', user.id);
    requestHeaders.set('x-publisher-id', user.publisherId);

    return NextResponse.next({
      request: {
        headers: requestHeaders,
      },
    });
  }

  return NextResponse.next();
}

export const config = {
  matcher: ['/((?!_next/static|_next/image|favicon.ico).*)'],
};
```

### Backend Authentication

Backend already implements JWT authentication in `src/middleware/auth.ts`:

```typescript
// backend/src/middleware/auth.ts (existing)
export const authenticate = async (req: Request, res: Response, next: NextFunction) => {
  try {
    const token = req.headers.authorization?.split(' ')[1];
    if (!token) {
      throw new AppError('No token provided', 401);
    }

    const decoded = jwt.verify(token, process.env.JWT_SECRET!);
    req.user = decoded; // { userId, publisherId, email, role }
    next();
  } catch (error) {
    next(new AppError('Invalid token', 401));
  }
};
```

---

## Dashboard Features

### 1. Revenue Dashboard

**URL:** `/dashboard/revenue`

**Features:**
- Total revenue today/week/month/all-time
- Revenue chart (time series)
- Top earning apps
- Revenue by network
- Revenue by country
- Revenue by ad format
- Export data (CSV, Excel, PDF)

**API Integration:**

```typescript
// website/src/app/api/revenue/summary/route.ts
import { NextRequest, NextResponse } from 'next/server';
import { getSession } from '@/lib/auth';

export async function GET(request: NextRequest) {
  const user = await getSession();
  if (!user) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }

  const { searchParams } = new URL(request.url);
  const startDate = searchParams.get('startDate');
  const endDate = searchParams.get('endDate');

  const response = await fetch(
    `${process.env.BACKEND_URL}/api/v1/revenue/summary?` +
    `startDate=${startDate}&endDate=${endDate}`,
    {
      headers: {
        Authorization: `Bearer ${user.id}`, // In production, use actual token
      },
    }
  );

  const data = await response.json();
  return NextResponse.json(data);
}
```

**Component:**

```typescript
// website/src/app/dashboard/revenue/page.tsx
'use client';

import { useState, useEffect } from 'react';
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, Legend } from 'recharts';

interface RevenueSummary {
  today: number;
  yesterday: number;
  thisWeek: number;
  thisMonth: number;
  allTime: number;
  change24h: number;
}

export default function RevenueDashboard() {
  const [summary, setSummary] = useState<RevenueSummary | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch('/api/revenue/summary')
      .then(res => res.json())
      .then(data => {
        setSummary(data.data);
        setLoading(false);
      });
  }, []);

  if (loading) return <div>Loading...</div>;

  return (
    <div className="p-8">
      <h1 className="text-3xl font-bold mb-8">Revenue Dashboard</h1>

      {/* Revenue Cards */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-6 mb-8">
        <RevenueCard
          title="Today"
          amount={summary?.today || 0}
          change={summary?.change24h || 0}
        />
        <RevenueCard
          title="This Week"
          amount={summary?.thisWeek || 0}
        />
        <RevenueCard
          title="This Month"
          amount={summary?.thisMonth || 0}
        />
        <RevenueCard
          title="All Time"
          amount={summary?.allTime || 0}
        />
      </div>

      {/* Revenue Chart */}
      <div className="bg-white rounded-lg shadow p-6">
        <h2 className="text-xl font-semibold mb-4">Revenue Trend</h2>
        <RevenueChart />
      </div>

      {/* Top Apps */}
      <div className="mt-8 bg-white rounded-lg shadow p-6">
        <h2 className="text-xl font-semibold mb-4">Top Earning Apps</h2>
        <TopAppsTable />
      </div>
    </div>
  );
}
```

### 2. Analytics Dashboard

**URL:** `/dashboard/analytics`

**Features:**
- Impressions, clicks, revenue metrics
- eCPM, CTR, fill rate
- Performance by adapter
- Performance by placement
- Performance by country
- Custom date range selection
- Real-time stats (last 5 minutes)

**Backend APIs (existing):**
- `GET /api/v1/analytics/overview` - Overview metrics
- `GET /api/v1/analytics/timeseries` - Time series data
- `GET /api/v1/analytics/performance?groupBy=adapter|placement` - Performance breakdown

### 3. Ad Networks Management

**URL:** `/dashboard/networks`

**Features:**
- View all integrated networks
- Add new network
- Configure network credentials
- Set floor prices per network
- Enable/disable networks
- View network performance
- Test network integration

**API Integration:**

```typescript
// website/src/app/api/adapters/route.ts
export async function GET() {
  const user = await getSession();
  if (!user) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

  const response = await fetch(
    `${process.env.BACKEND_URL}/api/v1/adapters`,
    {
      headers: { Authorization: `Bearer ${user.id}` },
    }
  );

  return NextResponse.json(await response.json());
}

export async function POST(request: NextRequest) {
  const user = await getSession();
  if (!user) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

  const body = await request.json();

  const response = await fetch(
    `${process.env.BACKEND_URL}/api/v1/adapters`,
    {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${user.id}`,
      },
      body: JSON.stringify(body),
    }
  );

  return NextResponse.json(await response.json());
}
```

**Component:**

```typescript
// website/src/app/dashboard/networks/page.tsx
'use client';

import { useState, useEffect } from 'react';

interface Adapter {
  id: string;
  name: string;
  status: 'active' | 'inactive' | 'testing';
  revenue24h: number;
  impressions24h: number;
  ecpm: number;
  fillRate: number;
}

export default function NetworksPage() {
  const [adapters, setAdapters] = useState<Adapter[]>([]);

  useEffect(() => {
    fetch('/api/adapters')
      .then(res => res.json())
      .then(data => setAdapters(data.data));
  }, []);

  return (
    <div className="p-8">
      <div className="flex justify-between items-center mb-8">
        <h1 className="text-3xl font-bold">Ad Networks</h1>
        <button
          className="bg-blue-600 text-white px-6 py-2 rounded-lg hover:bg-blue-700"
          onClick={() => {/* Open add network modal */}}
        >
          + Add Network
        </button>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {adapters.map(adapter => (
          <NetworkCard key={adapter.id} adapter={adapter} />
        ))}
      </div>
    </div>
  );
}
```

### 4. A/B Testing Dashboard

**URL:** `/dashboard/ab-tests`

**Features:**
- View all experiments
- Create new experiment
- Start/stop experiments
- View experiment results
- Statistical significance testing
- Thompson Sampling recommendations
- Export results

**Backend APIs (existing):**
- `POST /api/v1/ab-tests` - Create experiment
- `GET /api/v1/ab-tests` - List experiments
- `GET /api/v1/ab-tests/:id` - Get experiment details
- `POST /api/v1/ab-tests/:id/start` - Start experiment
- `POST /api/v1/ab-tests/:id/stop` - Stop experiment
- `POST /api/v1/ab-tests/:id/events` - Record event
- `GET /api/v1/ab-tests/:id/significance` - Test significance
- `GET /api/v1/ab-tests/:id/bandit` - Get bandit recommendation

**Component:**

```typescript
// website/src/app/dashboard/ab-tests/page.tsx
'use client';

import { useState, useEffect } from 'react';

interface Experiment {
  id: string;
  name: string;
  type: string;
  status: 'draft' | 'running' | 'paused' | 'completed';
  startDate: string;
  variants: Array<{
    id: string;
    name: string;
    trafficAllocation: number;
    metrics: {
      impressions: number;
      revenue: number;
      ecpm: number;
    };
  }>;
}

export default function ABTestsPage() {
  const [experiments, setExperiments] = useState<Experiment[]>([]);

  useEffect(() => {
    fetch('/api/ab-tests')
      .then(res => res.json())
      .then(data => setExperiments(data.data));
  }, []);

  return (
    <div className="p-8">
      <div className="flex justify-between items-center mb-8">
        <h1 className="text-3xl font-bold">A/B Tests</h1>
        <button
          className="bg-blue-600 text-white px-6 py-2 rounded-lg hover:bg-blue-700"
          onClick={() => {/* Open create experiment modal */}}
        >
          + Create Experiment
        </button>
      </div>

      <div className="space-y-6">
        {experiments.map(experiment => (
          <ExperimentCard
            key={experiment.id}
            experiment={experiment}
            onStart={() => handleStart(experiment.id)}
            onStop={() => handleStop(experiment.id)}
          />
        ))}
      </div>
    </div>
  );
}
```

### 5. Fraud Detection Dashboard

**URL:** `/dashboard/fraud`

**Features:**
- Real-time fraud rate
- Fraud types breakdown (click, install, impression, attribution, SDK)
- Blocked impressions
- Fraud score distribution
- Geographic fraud analysis
- Device/IP reputation scores
- Fraud trend chart
- Blocked IPs/devices list

**API Integration:**

```typescript
// website/src/app/api/fraud/stats/route.ts
export async function GET(request: NextRequest) {
  const user = await getSession();
  if (!user) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

  const { searchParams } = new URL(request.url);
  const startDate = searchParams.get('startDate');
  const endDate = searchParams.get('endDate');

  const response = await fetch(
    `${process.env.BACKEND_URL}/api/v1/fraud/stats?` +
    `startDate=${startDate}&endDate=${endDate}`,
    {
      headers: { Authorization: `Bearer ${user.id}` },
    }
  );

  return NextResponse.json(await response.json());
}
```

### 6. App Management

**URL:** `/dashboard/apps`

**Features:**
- View all apps
- Add new app
- Configure app settings
- SDK integration instructions
- App-level analytics
- App-level ad placements
- Test mode toggle

**Backend APIs:**
- `GET /api/v1/apps` - List apps
- `POST /api/v1/apps` - Create app
- `GET /api/v1/apps/:id` - Get app details
- `PUT /api/v1/apps/:id` - Update app
- `DELETE /api/v1/apps/:id` - Delete app

### 7. Ad Placements

**URL:** `/dashboard/placements`

**Features:**
- View all placements
- Create placement
- Configure floor prices
- Set mediation strategy (waterfall/header bidding)
- Configure network priority
- Placement-level analytics

**Backend APIs (existing):**
- `GET /api/v1/placements` - List placements
- `POST /api/v1/placements` - Create placement
- `GET /api/v1/placements/:id` - Get placement details
- `PUT /api/v1/placements/:id` - Update placement

### 8. Settings

**URL:** `/dashboard/settings`

**Features:**
- Account settings (email, password, API keys)
- Billing information
- Payment methods
- Invoice history
- Notification preferences
- Team management (add/remove users)
- Webhook configuration
- GDPR data export/deletion

---

## API Endpoints

### Website API Routes (Next.js)

All routes proxy to backend with authentication:

```
/api/auth/login             POST   - User login
/api/auth/logout            POST   - User logout
/api/auth/signup            POST   - User registration
/api/auth/me                GET    - Get current user

/api/revenue/summary        GET    - Revenue overview
/api/revenue/timeseries     GET    - Revenue time series
/api/revenue/export         GET    - Export revenue data

/api/analytics/overview     GET    - Analytics overview
/api/analytics/timeseries   GET    - Analytics time series
/api/analytics/performance  GET    - Performance metrics

/api/adapters               GET    - List adapters
/api/adapters               POST   - Add adapter
/api/adapters/:id           GET    - Get adapter details
/api/adapters/:id           PUT    - Update adapter
/api/adapters/:id/test      POST   - Test adapter

/api/ab-tests               GET    - List experiments
/api/ab-tests               POST   - Create experiment
/api/ab-tests/:id           GET    - Get experiment
/api/ab-tests/:id/start     POST   - Start experiment
/api/ab-tests/:id/stop      POST   - Stop experiment
/api/ab-tests/:id/results   GET    - Get results

/api/fraud/stats            GET    - Fraud statistics
/api/fraud/blocked          GET    - Blocked IPs/devices

/api/apps                   GET    - List apps
/api/apps                   POST   - Create app
/api/apps/:id               GET    - Get app details
/api/apps/:id               PUT    - Update app

/api/placements             GET    - List placements
/api/placements             POST   - Create placement
/api/placements/:id         GET    - Get placement
/api/placements/:id         PUT    - Update placement

/api/webhooks               GET    - List webhooks
/api/webhooks               POST   - Create webhook
/api/webhooks/:id           DELETE - Delete webhook

/api/settings/profile       GET    - Get profile
/api/settings/profile       PUT    - Update profile
/api/settings/api-keys      GET    - List API keys
/api/settings/api-keys      POST   - Create API key
```

### Backend API Routes (Existing)

All these routes already exist in the backend:

```
/api/v1/auth/login          POST   - Authenticate user
/api/v1/auth/register       POST   - Register new user

/api/v1/revenue/summary     GET    - Revenue summary
/api/v1/revenue/breakdown   GET    - Revenue breakdown

/api/v1/analytics/overview  GET    - Analytics overview
/api/v1/analytics/timeseries GET   - Time series data
/api/v1/analytics/performance GET  - Performance metrics

/api/v1/ab-tests            POST   - Create experiment
/api/v1/ab-tests            GET    - List experiments
/api/v1/ab-tests/:id        GET    - Get experiment
/api/v1/ab-tests/:id/start  POST   - Start experiment
/api/v1/ab-tests/:id/stop   POST   - Stop experiment
/api/v1/ab-tests/:id/events POST   - Record event
/api/v1/ab-tests/:id/significance GET - Test significance
/api/v1/ab-tests/:id/bandit GET    - Bandit recommendation

/api/v1/fraud/check         POST   - Check fraud
/api/v1/fraud/stats         GET    - Fraud statistics
/api/v1/fraud/report        POST   - Report fraud

/api/v1/adapters            GET    - List adapters
/api/v1/adapters/:id        GET    - Get adapter
/api/v1/adapters/:id/config PUT    - Update adapter config

/api/v1/placements          GET    - List placements
/api/v1/placements          POST   - Create placement
/api/v1/placements/:id      GET    - Get placement
/api/v1/placements/:id      PUT    - Update placement

/api/v1/rtb/bid             POST   - RTB bid request (SDK)
/api/v1/rtb/impression      POST   - Record impression (SDK)
/api/v1/rtb/click           POST   - Record click (SDK)
```

---

## Real-Time Features

### WebSocket Connection

Implement WebSocket for real-time dashboard updates:

```typescript
// website/src/lib/websocket.ts
import { useEffect, useState } from 'react';

export function useRealtimeStats() {
  const [stats, setStats] = useState(null);

  useEffect(() => {
    const ws = new WebSocket(`${process.env.NEXT_PUBLIC_WS_URL}/realtime`);

    ws.onmessage = (event) => {
      const data = JSON.parse(event.data);
      setStats(data);
    };

    return () => ws.close();
  }, []);

  return stats;
}
```

**Usage:**

```typescript
// website/src/app/dashboard/page.tsx
'use client';

import { useRealtimeStats } from '@/lib/websocket';

export default function DashboardPage() {
  const realtimeStats = useRealtimeStats();

  return (
    <div>
      <h2>Real-Time Stats</h2>
      <p>Revenue (last 5 min): ${realtimeStats?.revenue || 0}</p>
      <p>Impressions (last 5 min): {realtimeStats?.impressions || 0}</p>
      <p>Active users: {realtimeStats?.activeUsers || 0}</p>
    </div>
  );
}
```

### Server-Sent Events (SSE)

Alternative to WebSocket for one-way real-time updates:

```typescript
// website/src/app/api/realtime/route.ts
export async function GET() {
  const encoder = new TextEncoder();

  const stream = new ReadableStream({
    async start(controller) {
      while (true) {
        // Fetch latest stats from backend
        const response = await fetch(
          `${process.env.BACKEND_URL}/api/v1/analytics/realtime`
        );
        const data = await response.json();

        // Send to client
        controller.enqueue(
          encoder.encode(`data: ${JSON.stringify(data)}\n\n`)
        );

        // Wait 5 seconds
        await new Promise(resolve => setTimeout(resolve, 5000));
      }
    },
  });

  return new Response(stream, {
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    },
  });
}
```

---

## Data Models

### Frontend Types

```typescript
// website/src/types/index.ts

export interface User {
  id: string;
  email: string;
  publisherId: string;
  role: 'admin' | 'developer' | 'viewer';
  createdAt: string;
}

export interface App {
  id: string;
  name: string;
  bundleId: string;
  platform: 'ios' | 'android' | 'unity';
  status: 'active' | 'inactive';
  apiKey: string;
  createdAt: string;
}

export interface Placement {
  id: string;
  appId: string;
  name: string;
  adFormat: 'banner' | 'interstitial' | 'rewarded' | 'native';
  floorPrice: number;
  mediationStrategy: 'waterfall' | 'header_bidding';
  status: 'active' | 'inactive';
}

export interface Adapter {
  id: string;
  name: string;
  type: 'admob' | 'meta' | 'unity' | 'applovin' | 'ironsource';
  status: 'active' | 'inactive' | 'testing';
  credentials: Record<string, string>;
  priority: number;
  revenue24h: number;
  impressions24h: number;
  ecpm: number;
  fillRate: number;
}

export interface Experiment {
  id: string;
  name: string;
  description: string;
  type: 'floor_price' | 'adapter_priority' | 'placement_optimization' | 'waterfall_order';
  status: 'draft' | 'running' | 'paused' | 'completed';
  startDate: string;
  endDate?: string;
  variants: ExperimentVariant[];
  targetSampleSize: number;
  confidenceLevel: number;
}

export interface ExperimentVariant {
  id: string;
  name: string;
  trafficAllocation: number;
  configuration: Record<string, any>;
  metrics: {
    impressions: number;
    revenue: number;
    clicks: number;
    conversions: number;
    ecpm: number;
    ctr: number;
    conversionRate: number;
  };
}

export interface FraudStats {
  totalImpressions: number;
  fraudulentImpressions: number;
  fraudRate: number;
  blockedImpressions: number;
  savingsUsd: number;
  fraudTypes: {
    clickFraud: number;
    installFraud: number;
    impressionFraud: number;
    attributionFraud: number;
    sdkSpoofing: number;
  };
}

export interface RevenueMetrics {
  today: number;
  yesterday: number;
  thisWeek: number;
  thisMonth: number;
  allTime: number;
  change24h: number;
  changeWeek: number;
  changeMonth: number;
}

export interface AnalyticsOverview {
  impressions: number;
  clicks: number;
  revenue: number;
  ecpm: number;
  ctr: number;
  fillRate: number;
  avgLatency: number;
}
```

---

## Implementation Tasks

### Phase 1: Authentication & Basic Dashboard (1-2 weeks)

1. **Authentication System**
   - [ ] Implement JWT authentication in website
   - [ ] Create login/signup pages
   - [ ] Implement middleware for protected routes
   - [ ] Add session management (cookies)
   - [ ] Create auth API routes (`/api/auth/*`)
   - [ ] Test authentication flow

2. **Dashboard Layout**
   - [ ] Create dashboard layout component
   - [ ] Implement navigation sidebar
   - [ ] Add top navigation bar with user menu
   - [ ] Create breadcrumb navigation
   - [ ] Add responsive mobile layout

3. **Revenue Dashboard**
   - [ ] Create revenue API routes
   - [ ] Implement revenue summary cards
   - [ ] Add revenue time series chart
   - [ ] Create top apps table
   - [ ] Add revenue by network breakdown
   - [ ] Implement date range selector

### Phase 2: Analytics & Networks (2-3 weeks)

4. **Analytics Dashboard**
   - [ ] Create analytics API routes
   - [ ] Implement overview metrics cards
   - [ ] Add performance charts (impressions, clicks, eCPM)
   - [ ] Create adapter performance table
   - [ ] Add country breakdown map
   - [ ] Implement real-time stats widget

5. **Ad Networks Management**
   - [ ] Create adapters API routes
   - [ ] Implement network list view
   - [ ] Add "Add Network" modal with forms
   - [ ] Create network configuration UI
   - [ ] Add network testing functionality
   - [ ] Implement network performance cards

6. **App Management**
   - [ ] Create apps API routes
   - [ ] Implement app list view
   - [ ] Add "Create App" modal
   - [ ] Show SDK integration instructions
   - [ ] Add app-level analytics
   - [ ] Create API key generation

### Phase 3: Advanced Features (2-3 weeks)

7. **A/B Testing Dashboard**
   - [ ] Create A/B tests API routes
   - [ ] Implement experiment list view
   - [ ] Add "Create Experiment" wizard
   - [ ] Show experiment results with charts
   - [ ] Implement statistical significance testing
   - [ ] Add Thompson Sampling recommendations
   - [ ] Create experiment comparison view

8. **Fraud Detection Dashboard**
   - [ ] Create fraud API routes
   - [ ] Implement fraud statistics cards
   - [ ] Add fraud type breakdown chart
   - [ ] Show blocked IPs/devices list
   - [ ] Create fraud score distribution chart
   - [ ] Add geographic fraud analysis map

9. **Ad Placements Management**
   - [ ] Create placements API routes
   - [ ] Implement placement list view
   - [ ] Add "Create Placement" form
   - [ ] Configure mediation strategy per placement
   - [ ] Set floor prices per placement
   - [ ] Show placement-level analytics

### Phase 4: Settings & Advanced (1-2 weeks)

10. **Settings Pages**
    - [ ] Create profile settings page
    - [ ] Implement API key management
    - [ ] Add billing information page
    - [ ] Create payment methods management
    - [ ] Show invoice history
    - [ ] Implement team management
    - [ ] Add webhook configuration
    - [ ] Create notification preferences

11. **Real-Time Features**
    - [ ] Implement WebSocket/SSE for real-time updates
    - [ ] Add real-time revenue ticker
    - [ ] Show live impression count
    - [ ] Display active users count
    - [ ] Create real-time alerts for fraud/errors

12. **Data Export & Reporting**
    - [ ] Add CSV export for all data tables
    - [ ] Implement Excel export with formatting
    - [ ] Create PDF report generation
    - [ ] Add scheduled reports (email)
    - [ ] Implement custom report builder

### Phase 5: Polish & Optimization (1 week)

13. **Performance Optimization**
    - [ ] Implement React Query for data caching
    - [ ] Add loading skeletons
    - [ ] Optimize bundle size
    - [ ] Add service worker for offline support
    - [ ] Implement pagination for large lists

14. **UI/UX Improvements**
    - [ ] Add animations (Framer Motion)
    - [ ] Implement dark mode
    - [ ] Add keyboard shortcuts
    - [ ] Create onboarding tour
    - [ ] Add contextual help tooltips

15. **Testing & Documentation**
    - [ ] Write unit tests (Jest + React Testing Library)
    - [ ] Add E2E tests (Playwright)
    - [ ] Create API documentation
    - [ ] Write user guide
    - [ ] Add developer documentation

---

## Development Timeline

**Total Estimated Time: 8-11 weeks**

| Phase | Tasks | Duration | Dependencies |
|-------|-------|----------|--------------|
| Phase 1 | Auth & Basic Dashboard | 1-2 weeks | None |
| Phase 2 | Analytics & Networks | 2-3 weeks | Phase 1 |
| Phase 3 | Advanced Features | 2-3 weeks | Phase 1 |
| Phase 4 | Settings & Advanced | 1-2 weeks | Phase 2, 3 |
| Phase 5 | Polish & Optimization | 1 week | All phases |

**Minimum Viable Product (MVP):** Phases 1-2 (3-5 weeks)
- Authentication
- Basic dashboard with revenue/analytics
- App management
- Network configuration

**Full Feature Release:** All phases (8-11 weeks)

---

## Technology Stack

**Frontend:**
- Next.js 14 (App Router)
- TypeScript
- Tailwind CSS
- React Query (data fetching)
- Recharts (charts)
- Framer Motion (animations)
- Zod (validation)
- date-fns (date formatting)

**Backend (Existing):**
- Node.js + TypeScript
- Express.js
- PostgreSQL (user data, configs)
- Postgres analytics replicas (impressions, revenue, telemetry)
- Redis (caching, sessions)
- Go microservices (auction, fraud)

**Deployment:**
- Hetzner (PM2) for backend/console/website
- Caddy (TLS reverse proxy)
- Cloudflare (DNS/CDN)

---

## Security Considerations

1. **Authentication:**
   - JWT tokens with 7-day expiration
   - httpOnly cookies to prevent XSS
   - Refresh token rotation
   - Rate limiting on login attempts

2. **Authorization:**
   - Role-based access control (admin, developer, viewer)
   - API key authentication for programmatic access
   - Publisher-level data isolation

3. **Data Protection:**
   - HTTPS everywhere
   - Input validation (Zod)
   - SQL injection prevention (parameterized queries)
   - XSS prevention (React auto-escaping)
   - CSRF protection (SameSite cookies)

4. **API Security:**
   - Rate limiting (per user, per IP)
   - Request size limits
   - CORS configuration
   - API versioning

---

## Monitoring & Observability

1. **Application Monitoring:**
   - Vercel Analytics
   - Sentry (error tracking)
   - LogRocket (session replay)

2. **Performance Monitoring:**
   - Web Vitals (LCP, FID, CLS)
   - API response times
   - Database query performance

3. **Business Metrics:**
   - Daily Active Users (DAU)
   - User engagement (session duration)
   - Feature usage analytics
   - Conversion funnel (signup → first app → first revenue)

---

## Next Steps

1. **Review this design document** with stakeholders
2. **Prioritize features** based on business value
3. **Set up development environment**
4. **Begin Phase 1 implementation** (Auth & Basic Dashboard)
5. **Iterate based on user feedback**

---

## Questions & Decisions Needed

1. **Authentication:** Single sign-on (SSO) with Google/GitHub? Or email/password only?
2. **Billing:** Integrate Stripe for payment processing?
3. **Team Features:** Support multiple users per publisher account?
4. **Webhooks:** Real-time webhook delivery or queued?
5. **Export Limits:** Restrict data export size for non-enterprise users?
6. **Mobile App:** Build mobile app for dashboard (React Native)?

---

**Document Status:** DRAFT
**Last Updated:** 2025-11-04
**Author:** AI Assistant
**Reviewers:** [Pending]
