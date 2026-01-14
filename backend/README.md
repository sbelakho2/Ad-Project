# ApexMediation Backend

Core API service for the ApexMediation platform. Built with Node.js, Express, and TypeScript.

## 🏗️ Architecture

The backend follows a service-repository pattern:

- **Controllers**: HTTP entry points, request validation, and response mapping.
- **Services**: Business logic, auction orchestration, and external integrations.
- **Repositories**: Data access layer using PostgreSQL and Redis.
- **Middleware**: Authentication, rate limiting, logging, and error handling.

## 🛠️ Tech Stack

- **Runtime**: Node.js 20+
- **Framework**: Express.js
- **Language**: TypeScript 5.5
- **Database**: PostgreSQL (via `pg`)
- **Cache**: Redis (via `ioredis`)
- **Queues**: BullMQ
- **Monitoring**: Prometheus (RED metrics)
- **Documentation**: OpenAPI 3.0 / Swagger

## 🚀 Key Modules

### RTB Engine (`/src/services/rtb`)
High-performance auction orchestrator supporting parallel bidding with sub-100ms latency targets. Supports waterfall and S2S bidding with signed impression/click tokens.

### Fraud Detection (`/src/services/inference`)
Integrated with ML inference services. Supports **Shadow Mode** (logging only) and **Blocking Mode** (real-time mitigation).

### Analytics & Reporting (`/src/services/analytics`)
Rollup and aggregation logic for publisher revenue, impression tracking, and fill rates.

### Billing & Payouts (`/src/services/billing`)
Stripe integration for publisher billing and automated payout management with ledger reconciliation.

## 🚦 Getting Started

### Prerequisites
- PostgreSQL 16
- Redis 7
- Go 1.21 (for Go-based auction components if applicable)

### Installation
```bash
cd backend
npm install
```

### Environment Setup
Copy `.env.example` to `.env` and fill in the required values.

### Database Migrations
```bash
npm run migrate
```

### Development
```bash
npm run dev
```

### Testing
```bash
# Run all tests
npm run test:backend

# Run specific test file
npm run test:backend src/services/rtb/__tests__/orchestrator.test.ts
```

## 📈 Monitoring

The service exposes Prometheus metrics at `/metrics` and health checks at `/health`.

- `http_requests_total`: Total request count by method, route, and status.
- `http_request_duration_seconds`: Request latency histogram.
- `rtb_auction_latency_seconds`: RTB-specific latency metrics.
- `fraud_drift_psi`: ML model drift indicators.

## 🛡️ Security

- **TLS**: Strictly enforced for DB/Redis in production.
- **CORS**: Allowlisted origins for console and main website.
- **Auth**: JWT-based authentication with refresh token rotation.
- **Rate Limiting**: Tiered limiting based on publisher plan.
