# Backend Integration Guide

This document explains how the console frontend connects to the backend API.

## Configuration

### Environment Variables

The console uses environment variables to configure API connections. Create a `.env.local` file:

```bash
# Disable mock API to use real backend
NEXT_PUBLIC_USE_MOCK_API=false

# Backend API URLs (services may run on different hosts/ports)
NEXT_PUBLIC_API_URL=http://localhost:8080/api/v1
NEXT_PUBLIC_FRAUD_API_URL=http://localhost:8080/api/v1
NEXT_PUBLIC_ANALYTICS_API_URL=http://localhost:8080/api/v1

# NextAuth configuration
NEXTAUTH_URL=http://localhost:3000
NEXTAUTH_SECRET=your-secret-key-change-in-production

# Feature flags
NEXT_PUBLIC_ENABLE_ANALYTICS=true
NEXT_PUBLIC_ENABLE_FRAUD_DETECTION=true
```

### Backend Requirements
Each service can sit behind its own host/port combination. The example above matches the default Docker Compose stack where everything is exposed via the backend container on `localhost:8080`. If you split services across hosts, keep the `/api/v1` suffix and update the host/port for each `NEXT_PUBLIC_*_API_URL` accordingly.

Make sure your `.env.local` entries point to the correct hosts for your environment (Hetzner, Docker, or local dev). Supporting infrastructure usually includes:
- PostgreSQL (port 5432)
- Redis (port 6379)

Analytics queries now read exclusively from the Postgres analytics tables (backed by read replicas), so no ClickHouse service or credentials are required for local or remote environments.

## API Endpoints

All API endpoints are accessed through `/api/v1/*` base path:

### Authentication
- `POST /auth/login` - User login
- `POST /auth/register` - User registration
- `POST /auth/refresh` - Refresh access token

### Publishers
- `GET /publishers/me` - Get current publisher
- `PUT /publishers/me` - Update publisher profile

### Placements
- `GET /placements` - List placements (paginated)
- `GET /placements/:id` - Get placement by ID
- `POST /placements` - Create new placement
- `PUT /placements/:id` - Update placement
- `DELETE /placements/:id` - Delete placement

### Adapters
- `GET /adapters` - List adapters
- `GET /adapters/:id` - Get adapter by ID
- `POST /adapters` - Create adapter
- `PUT /adapters/:id` - Update adapter
- `DELETE /adapters/:id` - Delete adapter

### Revenue
- `GET /revenue/timeseries` - Get revenue time series data
- `GET /revenue/summary` - Get revenue summary

### Analytics
- `GET /analytics/impressions` - Get impressions data
- `GET /analytics/performance/:placementId` - Get placement performance
- `GET /analytics/overview` - Get analytics overview (cached)
- `GET /analytics/timeseries` - Get time series analytics (cached)
- `GET /analytics/performance` - Get performance metrics (cached)
- `POST /analytics/events/impressions` - Record impressions
- `POST /analytics/events/clicks` - Record clicks
- `POST /analytics/events/revenue` - Record revenue

### Fraud Detection
- `GET /fraud/alerts/:publisherId` - Get fraud alerts
- `GET /fraud/stats/:publisherId` - Get fraud statistics
- `GET /fraud/dashboard/:publisherId` - Get fraud dashboard
- `GET /fraud/trend/:publisherId` - Get fraud trends
- `GET /fraud/stats` - Get fraud stats
- `GET /fraud/alerts` - Get fraud alerts
- `GET /fraud/types` - Get fraud types

### Payouts
- `GET /payouts/history` - Get payout history (paginated)
- `GET /payouts/upcoming` - Get upcoming payout
- `PUT /payouts/method` - Update payout method

### Settings
- `GET /settings/fraud` - Get fraud settings
- `PUT /settings/fraud` - Update fraud settings
- `GET /settings/payout` - Get payout settings
- `PUT /settings/payout` - Update payout settings
- `GET /settings/notifications` - Get notification settings
- `PUT /settings/notifications` - Update notification settings
- `GET /settings/compliance` - Get compliance settings
- `PUT /settings/compliance` - Update compliance settings

### Team Management
- `GET /team/members` - List team members
- `POST /team/invite` - Invite team member
- `PUT /team/members/:id` - Update team member
- `DELETE /team/members/:id` - Remove team member
- `POST /team/members/:id/resend` - Resend invitation

### Background Jobs (Queues)
- `GET /queues/metrics` - Get all queue metrics
- `GET /queues/:queueName/metrics` - Get specific queue metrics
- `POST /queues/:queueName/jobs` - Add job to queue
- `GET /queues/:queueName/jobs/:jobId` - Get job status
- `DELETE /queues/:queueName/jobs/:jobId` - Cancel/remove job
- `POST /queues/:queueName/pause` - Pause queue
- `POST /queues/:queueName/resume` - Resume queue
- `POST /queues/:queueName/clean` - Clean old jobs

### Data Export
- `POST /data-export/jobs` - Create export job
- `GET /data-export/jobs/:jobId` - Get export job status
- `GET /data-export/jobs` - List export jobs
- `GET /data-export/jobs/:jobId/download` - Download export file
- `POST /data-export/warehouse/sync` - Schedule warehouse sync
- `POST /data-export/warehouse/sync/:syncId/execute` - Execute warehouse sync

### A/B Testing
- `GET /ab-testing/experiments` - List experiments
- `POST /ab-testing/experiments` - Create experiment
- `GET /ab-testing/experiments/:id` - Get experiment details
- `PUT /ab-testing/experiments/:id` - Update experiment
- `DELETE /ab-testing/experiments/:id` - Delete experiment
- `POST /ab-testing/experiments/:id/start` - Start experiment
- `POST /ab-testing/experiments/:id/stop` - Stop experiment
- `GET /ab-testing/experiments/:id/results` - Get experiment results
- `POST /ab-testing/events` - Record experiment event

## Authentication Flow

1. User enters credentials in login form
2. Console calls `POST /auth/login` on backend
3. Backend validates credentials and returns JWT token
4. Token is stored in NextAuth session
5. All subsequent API requests include `Authorization: Bearer <token>` header
6. Backend validates token on protected routes using `authenticate` middleware

## API Client Configuration

The console uses Axios with three client instances:

```typescript
import { apiClient, fraudApiClient, analyticsApiClient } from '@/lib/api-client'

// api-client.ts normalizes NEXT_PUBLIC_* env vars and appends /api/v1 if needed,
// so setting NEXT_PUBLIC_API_URL=http://localhost:8080 is enough for local stacks.

export const fetchPublisher = () => apiClient.get('/publishers/me')
export const fetchFraudStats = (publisherId: string) =>
  fraudApiClient.get(`/fraud/stats/${publisherId}`)
```
- Handle 401 errors by redirecting to login
- Format errors consistently

## Running the Stack

### 1. Start Backend Services

```bash
cd backend

# Start databases (if using Docker)
docker-compose up -d postgres redis

# Install dependencies
npm install

# Run migrations (if needed)
npm run migrate

# Start backend server
npm run dev
```

Backend will run on http://localhost:8080

### 2. Start Console

```bash
cd console

# Install dependencies
npm install

# Ensure .env.local has correct configuration
cat .env.local

# Start console
npm run dev
```

Console will run on http://localhost:3000

## Testing the Connection

1. Open browser to http://localhost:3000
2. Navigate to login page
3. Enter credentials or use demo mode
4. Check browser DevTools Network tab for API calls to http://localhost:8080
5. Verify responses contain real data from backend

## Switching Between Mock and Real API

Toggle the `NEXT_PUBLIC_USE_MOCK_API` environment variable:

```bash
# Use mock API (for frontend development without backend)
NEXT_PUBLIC_USE_MOCK_API=true

# Use real backend API
NEXT_PUBLIC_USE_MOCK_API=false
```

After changing this value, restart the console dev server.

## Caching

The backend implements Redis caching for GET endpoints with automatic cache invalidation on POST/PUT/DELETE operations. Cache TTL is typically 5 minutes for analytics data and 15 minutes for configuration data.

## Background Jobs

The backend uses BullMQ for background job processing:
- Analytics aggregation (daily at 1 AM)
- Data exports (on-demand and scheduled)
- Report generation (weekly on Monday at 8 AM)
- Metrics calculation (hourly)
- Cache warming (every 5 minutes)
- Cleanup (daily at 2 AM)

Monitor jobs via the `/queues/*` API endpoints.

## CORS Configuration

Backend CORS is configured to allow requests from http://localhost:3000 by default. Update `CORS_ORIGIN` in backend `.env` file for production deployments.

## Troubleshooting

### Console can't connect to backend

1. Check backend is running: `curl http://localhost:8080/health`
2. Verify `.env.local` has correct API URL
3. Check browser console for CORS errors
4. Ensure backend CORS_ORIGIN includes console URL

### Authentication fails

1. Check backend logs for authentication errors
2. Verify JWT_SECRET is set in backend `.env`
3. Check token is being sent in request headers
4. Verify user exists in database

### API returns 404

1. Verify endpoint path in `console/src/lib/api.ts`
2. Check backend route is registered in `backend/src/routes/index.ts`
3. Ensure no `/v1` prefix in API calls (already in base URL)

### Data not showing in console

1. Check browser DevTools Network tab for API responses
2. Verify backend has data (query databases directly)
3. Check console logs for parsing errors
4. Ensure data format matches TypeScript types

## Performance Considerations

- Backend caching reduces database load by 90-95%
- Analytics queries run on the partitioned Postgres analytics tables and replica pool
- Background jobs prevent blocking API requests
- Redis used for session storage and caching
- Connection pooling for PostgreSQL (PgBouncer)

## Security

- All backend routes (except auth/health) require JWT authentication
- Rate limiting: 100 requests per 15 minutes per IP
- CORS restricted to configured origins
- Request body size limited to 10MB
- Helmet.js security headers enabled
- Input validation on all endpoints
- SQL injection protection via parameterized queries
