# Ad Platform Console

_Last updated: 2025-12-12_

> **FIX-10 governance:** This README describes console features and architecture. For actual deployment readiness, see `docs/Internal/Deployment/PROJECT_STATUS.md`. For prioritized backlog (including console work), see `docs/Internal/Development/FIXES.md` (FIX-03).

## Overview

Modern Next.js 14 web application providing publishers with a comprehensive dashboard for ad mediation management, revenue analytics, fraud detection, and payout tracking.

## Tech Stack

- **Framework**: Next.js 14 (App Router)
- **UI**: React 18, TypeScript 5
- **Styling**: Tailwind CSS 3.3
- **State Management**: Zustand, TanStack Query (React Query)
- **Authentication**: NextAuth.js
- **Charts**: Recharts
- **Tables**: TanStack Table
- **Forms**: React Hook Form + Zod validation
- **HTTP Client**: Axios
- **Icons**: Lucide React

## Features

### Dashboard
- Live revenue, impression, and fill-rate widgets backed by backend `/api/v1` data via TanStack Query
- Time-series charts with locale-aware formatting and CSV export helpers
- Fraud & payout widgets sharing normalized status badges and CTA links

### SDK Keys Management (NEW - 2025-12-12)
- Per-app SDK key generation and management at `/settings/keys`
- Test (`sk_test_`) and live (`sk_live_`) key modes
- Key rotation with secure one-time secret display
- Key revocation with confirmation modal
- Copy-to-clipboard with toast notifications
- Integration with Go RTB services for SDK authentication

### Placement & Adapter Management
- Infinite-scroll placement list with client-side filters/search
- Placement creation flow covering format metadata, duplicate validation, and slug previews
- Adapter table with inline error states and instant navigation via prefetching

### Transparency & Verification
- Auctions list with debounced filters, URL syncing, Verify badge interactions, and manual refresh controls that only appear when `NEXT_PUBLIC_ENABLE_TRANSPARENCY_REFRESH=true`
- Auction detail view showing canonical payloads, signature metadata, and retryable verification
- Summary page surfacing sampling totals, publisher share averages, and refresh controls (also controlled by `NEXT_PUBLIC_ENABLE_TRANSPARENCY_REFRESH`)

### Fraud Detection & Analytics
- Fraud dashboard widgets sourced from live APIs with configurable severity thresholds
- Analytics views offering custom ranges, localized charts, and CSV exports

### Billing, Payouts & Admin Workflows
- Billing settings gated by feature flags with inline validation, toast persistence, and a beta Migration Assistant panel that appears when `NEXT_PUBLIC_ENABLE_BILLING_MIGRATION=true`
- Invoice downloads with TTL-bound cache + TanStack Query data access
- Admin health, billing ops, and audit surfaces protected by `useAdminGate`

## Project Structure

```
console/
├── src/
│   ├── app/                    # Next.js App Router
│   │   ├── api/               # API routes
│   │   │   └── auth/         # NextAuth configuration
│   │   ├── dashboard/        # Dashboard pages
│   │   ├── placements/       # Placement management
│   │   ├── fraud/            # Fraud detection
│   │   ├── payouts/          # Payout management
│   │   ├── settings/         # Account settings
│   │   ├── layout.tsx        # Root layout
│   │   ├── page.tsx          # Home page (redirects to dashboard)
│   │   ├── providers.tsx     # Context providers
│   │   └── globals.css       # Global styles
│   ├── components/           # React components
│   │   ├── ui/              # Reusable UI components
│   │   ├── charts/          # Chart components
│   │   ├── tables/          # Table components
│   │   └── forms/           # Form components
│   ├── lib/                 # Utilities
│   │   ├── api.ts          # API client functions
│   │   ├── api-client.ts   # Axios configuration
│   │   └── utils.ts        # Helper functions
│   ├── hooks/              # Custom React hooks
│   ├── types/              # TypeScript type definitions
│   └── store/              # Zustand stores
├── public/                 # Static assets
├── package.json
├── tsconfig.json
├── tailwind.config.ts
├── next.config.js
└── README.md
```

## Getting Started

### Prerequisites

- Node.js 18+ and npm/yarn/pnpm
- Backend services running (Config, Fraud, Analytics APIs)

### Installation

```bash
# Navigate to console directory
cd console

# Install dependencies
npm install

# Copy environment variables
cp .env.local.example .env.local

# Edit .env.local with your configuration
```

### Environment Variables

Create `.env.local`:

```env
# API Configuration
NEXT_PUBLIC_API_URL=http://localhost:8080
NEXT_PUBLIC_FRAUD_API_URL=http://localhost:8083
NEXT_PUBLIC_ANALYTICS_API_URL=http://localhost:8084
NEXT_PUBLIC_TRANSPARENCY_API_URL=http://localhost:8086

# Authentication
NEXTAUTH_URL=http://localhost:3000
NEXTAUTH_SECRET=your-secret-key-here

# OAuth Providers (optional)
GOOGLE_CLIENT_ID=
GOOGLE_CLIENT_SECRET=
GITHUB_CLIENT_ID=
GITHUB_CLIENT_SECRET=

# Feature Flags / Defaults
NEXT_PUBLIC_USE_MOCK_API=false
NEXT_PUBLIC_REQUIRE_CONSENT=false
NEXT_PUBLIC_DEFAULT_LOCALE=en-US
NEXT_PUBLIC_DEFAULT_CURRENCY=USD
NEXT_PUBLIC_ENABLE_TRANSPARENCY_REFRESH=true
NEXT_PUBLIC_ENABLE_BILLING_MIGRATION=false
NEXT_PUBLIC_REQUIRE_ADMIN_GUARD=true
```

Feature toggles mirror `.env.local.example`:

- `NEXT_PUBLIC_USE_MOCK_API` swaps network calls for fixtures.
- `NEXT_PUBLIC_ENABLE_TRANSPARENCY_REFRESH` controls whether manual refresh/retry buttons render on the Transparency Auctions + Summary pages (the queries still auto-refresh when disabled).
- `NEXT_PUBLIC_ENABLE_BILLING_MIGRATION` reveals the beta Migration Assistant card inside billing settings so teams can submit migration context to ops. Ensure the backend `POST /billing/migration/request` route (or an equivalent mock) is live before flipping this flag; the form fails hard on 404/500s.
- `NEXT_PUBLIC_REQUIRE_ADMIN_GUARD` enforces `useAdminGate` redirects; set it to `false` to bypass redirects while keeping the rest of the hook stateful for local debugging.

#### Migration Assistant prerequisites

1. Run the backend workspace (`npm run dev --workspace backend`) or deploy the billing service so that `POST /api/v1/billing/migration/request` is reachable (the console API client proxies that as `/billing/migration/request`).
2. If you do not have the backend handy, update `src/app/api/mock/route.ts` (or your MSW fixtures) to return a stub response for `POST /billing/migration/request` and launch the console with `NEXT_PUBLIC_USE_MOCK_API=true`.
3. Keep the `MIGRATION_NOTES_MIN_LENGTH` (20 chars) enforcement in mind when demoing—the UI disables submit and shows inline validation until the threshold is satisfied.

### Development

```bash
# Run development server
npm run dev

# Open browser to http://localhost:3000
```

### Building for Production

```bash
# Type checking
npm run type-check

# Linting
npm run lint

# Build
npm run build

# Start production server
npm start

# Clear build caches (.next + .swc) if you hit stale transpilation errors
npm run clean

### Storybook

- `npm run storybook` – starts the React/Webpack 5 Storybook instance configured in `.storybook/main.ts` with the shared Babel loader so component stories behave the same as Next.js pages (no Next cache hooks involved).
- `npm run storybook:build` – produces a static bundle in `console/storybook-static`. This now runs independently from Next build caches, but if you ever hit weird module resolution issues, run `npm run clean` first and then rebuild.
- Common troubleshooting: confirm the aliases in `.storybook/main.ts` match `tsconfig.json`, and remember Storybook won’t automatically stand up the backend. Toggle `NEXT_PUBLIC_USE_MOCK_API=true` or start the backend services when stories depend on live APIs.
```

## API Integration

The console integrates with multiple backend services:

### Configuration Service (Port 8080)
- `/v1/publishers/*` - Publisher management
- `/v1/placements/*` - Placement CRUD
- `/v1/adapters/*` - Adapter configuration
- `/v1/auth/*` - Authentication

### Fraud Service (Port 8083)
- `/v1/fraud/alerts/:publisherId` - Fraud alerts
- `/v1/fraud/stats/:publisherId` - Fraud statistics
- `/v1/fraud/dashboard/:publisherId` - Dashboard data
- `/v1/fraud/trend/:publisherId` - Trend analysis

### Analytics Service (Port 8084)
- `/v1/analytics/impressions` - Impression data
- `/v1/analytics/performance/:placementId` - Performance metrics
- `/v1/revenue/*` - Revenue analytics

## Authentication

Uses NextAuth.js with credential-based authentication:

1. User submits email/password
2. Credentials verified against backend API (`/v1/auth/login`)
3. JWT token stored in session
4. Token included in all API requests via Axios interceptors

### Protected Routes

All routes except `/login` require authentication. Unauthenticated users are redirected to login page.

## Styling

### Tailwind Utilities

Custom component classes defined in `globals.css`:

```css
.btn              # Base button
.btn-primary      # Primary button (blue)
.btn-secondary    # Secondary button (gray)
.btn-success      # Success button (green)
.btn-danger       # Danger button (red)
.btn-outline      # Outlined button

.card             # Card container
.input            # Form input
.label            # Form label

.badge            # Badge base
.badge-success    # Success badge
.badge-warning    # Warning badge
.badge-danger     # Danger badge
.badge-info       # Info badge
```

### Custom Colors

Extended Tailwind palette:
- `primary-*` (blue shades)
- `success-*` (green shades)
- `warning-*` (yellow shades)
- `danger-*` (red shades)

## Data Flow

1. **Query**: Components use TanStack Query hooks
2. **API Call**: Query functions call API endpoints via `lib/api.ts`
3. **HTTP**: Axios client (`lib/api-client.ts`) sends requests
4. **Interceptors**: Add auth token, handle errors
5. **Response**: Data returned to component
6. **Cache**: TanStack Query caches results (1 minute stale time)
7. **UI Update**: Component re-renders with new data

## Error Handling

- API errors caught by Axios interceptors
- 401 errors trigger logout and redirect to login
- User-friendly error messages displayed via toast notifications
- Form validation via Zod schemas

## Performance Optimizations

- Server-side rendering for initial page load
- Automatic code splitting (Next.js)
- Image optimization (next/image)
- React Query caching (1 minute default)
- Debounced search inputs
- Lazy loading for charts and heavy components

## Security

- CSRF protection via NextAuth
- HTTP-only cookies for session tokens
- XSS protection (React auto-escaping)
- Input validation (Zod schemas)
- Secure API communication (HTTPS in production)

## Client Observability & Debug Logging

Console noise is intentionally limited so critical signals surface quickly:

- The ESLint rule `no-console` blocks `console.log` and `console.info`; only `console.warn` (degraded UX) and `console.error` (fatal failures) are permitted in source code. Linting fails the build if other console methods slip through.
- Use `console.warn` for recoverable states (e.g., feature-flagged surfaces falling back to skeletons) and pair each call with a user-facing affordance such as toasts or inline banners.
- Reserve `console.error` for unrecoverable flows. The global error boundary (`src/app/error.tsx`) captures thrown client errors and forwards them to `/api/logs/client-error`, so additional errors should include actionable context (request IDs, feature flag values) instead of stack traces alone.
- For deep local debugging, wrap temporary logs in `if (process.env.NODE_ENV !== 'production')` and remove them before merging; linting ensures production builds stay clean.
- Telemetry destined for backend observability should go through dedicated APIs (e.g., `settingsApi`, `reportClientError`) instead of sprinkling console statements throughout components.

## Deployment

### Vercel (Recommended)

```bash
# Install Vercel CLI
npm i -g vercel

# Deploy
vercel
```

### Docker

```bash
# Build image
docker build -t ad-platform-console .

# Run container
docker run -p 3000:3000 ad-platform-console
```

### Fly.io

Fly is configured via `console/fly.toml` and deploys the production-ready Next.js server close to regional bidders.

```bash
# Authenticate once
fly auth login

# Set secrets for backend URLs and NextAuth session keys
fly secrets set NEXTAUTH_SECRET=... NEXT_PUBLIC_API_URL=...

# Deploy using the provided config (build uses Dockerfile)
fly deploy --config fly.toml --remote-only
```

Scaling, certificates, and log drains can then be managed through `fly scale`, `fly certs`, and `fly logs`.

### Manual Deployment

```bash
# Build
npm run build

# Start
npm start
```

## Browser Support

- Chrome 90+
- Firefox 88+
- Safari 14+
- Edge 90+

## Future Enhancements

1. **Real-time Updates**: WebSocket integration for live metrics
2. **Advanced Filters**: Complex filtering UI for all tables
3. **Report Scheduling**: Automated email reports
4. **Multi-language**: i18n support
5. **Dark Mode**: Theme switcher
6. **Mobile App**: React Native companion app
7. **Export Features**: PDF/Excel report generation
8. **Custom Dashboards**: Drag-and-drop dashboard builder
9. **Alerts**: Push notifications for critical events
10. **Admin Panel**: Multi-publisher management for platform admins

## License

Proprietary - © 2025 Bel Consulting OÜ. ApexMediation is a brand of Bel Consulting OÜ. All rights reserved.

## Support

For issues or questions, contact: support@adplatform.com
