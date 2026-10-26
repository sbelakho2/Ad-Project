Rival Apex Mediation Web SDK

_Last updated: 2025-11-18_

> **FIX-10 governance:** This README documents Web SDK capabilities. For SDK backlog and production readiness, see `docs/Internal/Deployment/PROJECT_STATUS.md` and `docs/Internal/Development/FIXES.md` (FIX-05).

A lightweight browser SDK for consent, auction requests, and ad delivery integration.

Public API
- init(options): initialize the SDK with endpoint and optional metadata.
- setConsent(consent): set/update consent flags (GDPR/TCF, CCPA/USP, GPP, COPPA).
- requestAd(request): POSTs to {endpoint}/auction with AbortController timeout and validates response.
- on(event, handler): subscribe to SDK events (consent:updated, ad:requested, ad:filled, ad:error).

Usage metering (billing)
- The SDK can post billable usage metrics to the API for Mollie usage sync.
- Endpoint: POST /api/v1/metrics/usage
- Authentication: X-Api-Key header with an app API key.
- Configure the SDK with reportEndpoint + apiKey if you enable remote reporting.

Build outputs
- ESM: dist/index.esm.js
- UMD: dist/index.umd.js
- Types: dist/src/index.d.ts

Commands
- npm run lint
- npm run build
- npm run test -- --coverage

Notes
- Fetch timeouts are enforced with AbortController (default 2000ms).
- Response validation uses zod to ensure schema correctness.
- Error taxonomy mirrors mobile SDKs (INIT_REQUIRED, TIMEOUT, NETWORK, BAD_RESPONSE, VALIDATION, NO_FILL, UNKNOWN).

Supported networks
- The Web SDK surfaces the list of parity networks with mobile/CTV via SUPPORTED_NETWORKS export.
- Programmatic API:
  - import { SUPPORTED_NETWORKS, getSupportedAdapters } from '@rivalapex/web-sdk'
  - getSupportedAdapters() returns an array of 15 network identifiers: admob, applovin, unity, ironsource, facebook, vungle, chartboost, pangle, mintegral, adcolony, tapjoy, moloco, fyber, smaato, amazon.
- See also: docs/Adapters/SUPPORTED_NETWORKS.md in the repository for cross-platform details and code locations.
