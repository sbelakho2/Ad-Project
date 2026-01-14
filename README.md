# ApexMediation Platform

[![CI](https://github.com/sbelakho2/apex-mediation-platform/actions/workflows/ci.yml/badge.svg)](https://github.com/sbelakho2/apex-mediation-platform/actions)
[![License](https://img.shields.io/badge/license-PROPRIETARY-red.svg)](LICENSE)

High-performance, reliable ad mediation platform designed to address critical industry failures. Built for transparency, performance, and solo-operator efficiency.

## 🚀 Key Features

- **OTA-Proof Reliability**: Cryptographically signed configurations with staged rollouts to prevent remote app breakage.
- **Ultra-Lean SDK**: <500KB footprint, <0.02% ANR contribution, and cold start in <100ms.
- **Full Transparency**: Per-impression bid landscapes and real-time fraud scoring (Shadow and Blocking modes).
- **Automated Payouts**: Weekly multi-rail payouts with 99.95% reliability.
- **Estonian Compliance**: Integrated e-government reporting for VAT and annual financial statements.

## 🏗️ Architecture Overview

The platform is a multi-service monorepo:

- **[Backend](./backend)**: Node.js/TypeScript Express API handling configuration, reporting, and management.
- **[Console](./console)**: Modern Next.js 14 dashboard for publishers to manage apps, placements, and view analytics.
- **[Control Plane](./control-plane)**: Internal management service for operator tasks, compliance reporting, and infrastructure monitoring.
- **[SDKs](./sdk)**: High-performance native and cross-platform SDKs (Android, iOS, Unity, Web, CTV).
- **[Services](./services)**: Specialized microservices including `fraud-inference` (Python/FastAPI) for real-time ML-based fraud detection.
- **[Infrastructure](./infrastructure)**: Terraform and Docker-based deployment configurations for Hetzner.

## 📁 Repository Structure

```text
.
├── backend/            # Main API service (Node.js/Express)
├── console/            # Publisher dashboard (Next.js)
├── control-plane/      # Internal operator service
├── sdk/                # Multi-platform mediation SDKs
│   ├── android/
│   ├── ios/
│   ├── unity/
│   └── web/
├── services/           # Microservices (Fraud Inference, etc.)
├── ML/                 # Machine Learning training pipelines
├── models/             # Versioned ML model artifacts (ONNX)
├── docs/               # Technical and business documentation
├── infrastructure/     # Deployment and IaC (Terraform, Docker)
├── scripts/            # Operational and automation scripts
└── tools/              # CI/CD and developer tools
```

## 🛠️ Getting Started

### Prerequisites
- Node.js 20+
- Docker & Docker Compose
- Go 1.21+ (for RTB engine)
- Python 3.11+ (for ML services)

### Quick Start
1. **Clone the repo**:
   ```bash
   git clone https://github.com/sbelakho2/apex-mediation-platform.git
   cd Ad-Project
   ```

2. **Install dependencies**:
   ```bash
   npm install
   ```

3. **Start development environment**:
   ```bash
   ./start-dev.sh
   ```
   *Note: This starts the console and backend services in development mode.*

4. **Verify Health**:
   ```bash
   curl http://localhost:8080/healthz  # Backend
   curl http://localhost:3000          # Console
   ```

For detailed development instructions, see the [Development Guide](./docs/Internal/Development/DEVELOPMENT.md).

## 📊 Project Status

Current status: **Production Ready** (Core Path)

| Component | Status | Verification |
| --- | --- | --- |
| RTB Auction | ✅ Stable | 100ms latency targets met, signed tokens verified |
| Fraud Detection | ✅ Blocking | Real-time ONNX inference with auto-blocking |
| Compliance | ✅ Active | Real Estonian ledger integration for VAT/Annual reports |
| Console | ✅ Connected | Live API integration with full App CRUD |
| SDKs | ✅ Shipped | Android/iOS/Unity parity achieved |

See the [Canonical Project Status](./docs/Internal/Deployment/PROJECT_STATUS.md) for a detailed breakdown.

## 📚 Documentation Map

- **[Architecture](./docs/Architecture)**: System design and SRS.
- **[Customer-Facing](./docs/Customer-Facing)**: Quickstart, API reference, and SDK integration.
- **[Internal Operations](./docs/Internal)**: Deployment guides, security audits, and financial compliance.
- **[Runbooks](./docs/Runbooks)**: Operational procedures for incident response and cost controls.

## 🛡️ License

Proprietary - © 2026 Bel Consulting OÜ. All rights reserved.
