# 🚀 ApexMediation - Enterprise Ad Mediation Platform
<!-- markdownlint-disable MD013 -->

_Last updated: 2025-11-18 16:30 UTC_

> **Reality check (FIX-10):** This README pairs high-level goals with the actual status recorded in `docs/Internal/Deployment/PROJECT_STATUS.md`. Treat that file plus `docs/Internal/Development/FIXES.md` as the authoritative sources before quoting capability or completion dates.

**Project stage:** In development; backend/console/website/SDK workstreams remain open per `PROJECT_STATUS.md`.

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Go Version](https://img.shields.io/badge/go-1.21+-blue.svg)](https://golang.org)
[![Node Version](https://img.shields.io/badge/node-20+-green.svg)](https://nodejs.org)

**Entity:** Bel Consulting OÜ (Estonia), daughter of Starz Energies (US)  
**Mission:** Build and deploy ApexMediation, an enterprise-grade ad mediation platform with OTA-proof reliability, transparent bid landscapes, multi-rail payments, and developer-first trust.

## 🎯 Project Overview

These goals target Unity's most visible Unity LevelPlay gaps:

- Signed config rollout with staged promotion to prevent OTA crashes.
- SDK architecture that keeps ANR contribution under 0.02%.
- Multi-rail payout orchestration with 99.95% reliability targets.
- Per-impression transparency (bid landscapes, verification flows).
- Open tooling and public metrics to rebuild trust.

Delivery status lives in the snapshot below and in `docs/Internal/Deployment/PROJECT_STATUS.md`.

## 📡 Current Delivery Snapshot (2025-11-18)

| Area | Legacy claim | Reality (see `PROJECT_STATUS.md`) | FIX coverage |
| --- | --- | --- | --- |
| Backend platform | "All services production-ready" | `backend/Dockerfile` broken; controllers missing auth/rate limits; billing/transparency APIs mid-remediation. | FIX-01 |
| Console + Admin UI | "Connected to live APIs with full RBAC" | Remaining mock flows for login, billing, migration studio; default nav exposes disabled routes. | FIX-03 |
| Website | "Data-backed dashboards" | Pages still render static metrics/demo funnels; compliance copy needs review. | FIX-04 |
| SDKs | "Android/iOS/Unity shipped" | iOS/Android parity work mid-stream; Unity lacks device validation; consent persistence unfinished. | FIX-05 |
| Documentation | "Comprehensive & current" | Index + summaries still reference obsolete TODO sources; legal/PCI docs missing ownership dates. | FIX-10 |

## 📁 Repository Structure

```text
/
├── sdk/                    # Mobile SDKs (Android, iOS, Unity)
├── backend/               # Backend services (Go, TypeScript, Python)
├── console/               # Publisher dashboard (Next.js)
├── infrastructure/        # Terraform + Kubernetes
├── data/                  # Schemas, migrations, analytics
├── quality/               # Testing & chaos engineering
├── docs/                  # Documentation
└── tools/                 # Migration & debugging tools
```

## 🚀 Quick Start

### Prerequisites

- Node.js >= 18
- Go >= 1.21
- Python >= 3.11
- Docker & Docker Compose
- Terraform >= 1.5
- kubectl >= 1.28

### Development Setup

```bash
# Clone repository
git clone https://github.com/sbelakho2/Ad-Project.git
cd Ad-Project

# Install dependencies
npm install

# Start development environment
docker-compose up -d

# Run tests
npm test
```

## 📚 Documentation

- `docs/INDEX.md` — master directory of every doc (update this before adding new folders).
- `docs/Internal/Deployment/PROJECT_STATUS.md` — canonical readiness narrative replacing legacy "project complete" files.
- `docs/Internal/Development/FIXES.md` — prioritized backlog (163 TODOs) plus FIX-10 change log.
- `docs/Internal/Development/AD_PROJECT_FILE_ANALYSIS.md` — file-by-file risk inventory that feeds each FIX.
- `docs/Internal/Deployment/PROJECT_COMPLETE.md` / `PROJECT_COMPLETION.md` / `SYSTEM_COMPLETE.md` — archived summaries kept only for historical context; each now links back to `PROJECT_STATUS.md`.

## 🏗️ Architecture Highlights

### Thread-Safe SDK

- All network I/O on background threads
- StrictMode enforcement (Android)
- Circuit breakers per adapter
- <500KB core SDK size

### Configuration Safety

- Ed25519 cryptographic signing
- Staged rollouts (1% → 5% → 25% → 100%)
- Automatic rollback on SLO breach
- Kill switches within 30 seconds

### Hybrid Auction

- Server-side bidding (primary)
- Header bidding (secondary)
- Waterfall fallback
- <100ms p99 latency

### Multi-Rail Payments

- Weekly payouts (vs Unity monthly)
- Multiple providers (Tipalti, Wise, Payoneer)
- Automatic failover
- Double-entry ledger

## 📊 Performance Targets

| Metric | Target | Unity Comparison |
| --- | --- | --- |
| SDK Size | <500KB | ~2MB |
| ANR Rate | <0.02% | >0.1% |
| Crash-Free | ≥99.9% | ~99% |
| Payment Success | ≥99.95% | ~98% |
| P99 Latency | ≤100ms | ~150ms |

## 🛠️ Development Roadmap

> **Planning source:** The phased outline below is historical context. Active scheduling, acceptance criteria, and progress tracking live in `docs/Internal/Development/FIXES.md` and `docs/Internal/Development/DEVELOPMENT_ROADMAP.md` (which now references `PROJECT_STATUS.md`).

### Phase 1: Foundation (Days 0-30)

- [x] Monorepo structure
- [ ] Core SDK architecture
- [ ] Configuration service
- [ ] Basic auction engine

### Phase 2: Privacy & Payments (Days 31-60)

- [ ] Privacy compliance (iOS ATT, Privacy Sandbox)
- [ ] Payment orchestration
- [ ] Multi-rail integration

### Phase 3: Intelligence & Scale (Days 61-90)

- [ ] Fraud detection (GIVT/SIVT)
- [ ] ML optimization
- [ ] Load testing (1M QPS)

### Phase 4: Production Ready (Days 91-120)

- [ ] Partner integrations
- [ ] Unity migration tools
- [ ] Beta program
- [ ] Public launch

## 🧪 Testing

```bash
# Unit tests
npm run test:unit

# Integration tests
npm run test:integration

# Load tests (K6)
npm run test:load

# Chaos engineering
npm run test:chaos
```

## 🔒 Security

- Ed25519 signing for configs
- AES-256-GCM encryption at rest
- TLS 1.3 for transit
- Certificate pinning in SDKs
- HSM for production keys

## 📈 Monitoring

- **Metrics:** Prometheus + Grafana
- **Logs:** Loki
- **Traces:** Jaeger
- **Alerts:** PagerDuty
- **Status:** <https://status.platform.com>

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

## 📄 License

Proprietary - Copyright © 2025 Bel Consulting OÜ

## 📞 Contact

- **Email:** <founders@platform.com>
- **Status Page:** <https://status.platform.com>
- **Developer Portal:** <https://developers.platform.com>
- **Discord:** <https://discord.gg/rival-ad-stack>

---

### Built with ❤️ by Bel Consulting OÜ

## 🗒️ Documentation Change Log

| Date | Change |
| --- | --- |
| 2025-11-18 | Added FIX-10 governance banner, delivery snapshot, updated documentation links, and roadmap note referencing `PROJECT_STATUS.md` + `FIXES.md`. |

<!-- markdownlint-enable MD013 -->
