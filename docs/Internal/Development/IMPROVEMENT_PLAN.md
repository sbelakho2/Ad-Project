### Improvement Plan — Ad-Project (ApexMediation)

#### 0) Purpose and scope
This document turns the current repository review into a concrete, actionable improvement plan. It focuses on first‑party code, configs, workflows, and docs across all packages. Generated/vendor directories (e.g., `node_modules/`, `.venv/`, `dist/`, `build/`) are excluded.

Outcomes:
- Close gaps in CI/CD, releases, observability, security/supply chain.
- Standardize SDK distribution across platforms.
- Improve documentation hygiene and operational readiness.

Deliverables are grouped by area with prioritized tasks, acceptance criteria, and suggested milestones.

---

#### 1) Highlights — What’s already strong
- Cohesive workspace structure (npm workspaces) with fan‑out scripts.
- Consolidated CI gate `ci-all.yml` covering backend (with Postgres/ClickHouse services), console, Android/iOS/CTV SDKs.
- Android SDK build: modern toolchain, R8/proguard, size gate (`checkSdkSize`), config validation (`validateSdkConfig`), Dokka integration.
- Monitoring stack: Prometheus, Grafana, Loki, Alertmanager; clear `monitoring/README.md`; initial alert rules present.
- Backend stack: Express/TypeScript, Postgres, Redis/BullMQ, ClickHouse, prom-client, Stripe/email; migration verification in CI.
- Console app: Next.js with type‑check and accessibility testing lane.
- CTV CI lanes (Android TV, tvOS) present.

---

#### 2) Cross‑cutting improvements (Priority A)
2.1 CI/CD consolidation and determinism
- [ ] Pick a single gate workflow; retire duplicates between `ci-all.yml` and any legacy `ci.yml`.  
  Acceptance: Only one PR gate required; docs updated (see 6.1).
- [ ] Unify SDK release workflows: choose between `.github/workflows/release-sdks.yml` and `.github/workflows/sdk-release.yml`.  
  Acceptance: One tag‑driven release pipeline builds/publishes all SDK artifacts; obsolete workflow removed or archived.
- [ ] Ensure caches are deterministic and keyed on lockfiles (npm, Gradle, Swift).  
  Acceptance: CI cache steps reference lockfiles and produce reproducible builds.

2.2 Security and supply chain
- [ ] Extend Trivy to scan built Docker images for backend/console and upload CycloneDX SBOMs.  
  Acceptance: CI uploads SBOM artifacts; CRITICAL issues block unless explicitly waived.
- [ ] Ensure CodeQL covers JS/TS and, where supported, Kotlin/Swift. Document limitations.  
  Acceptance: CodeQL runs on default branches and PRs; findings triaged.
- [ ] Optional: Add Snyk for Gradle/Swift/npm to complement CodeQL.  
  Acceptance: HIGH/CRITICAL findings labeled and optionally gated per policy.
- [ ] Secrets policy: establish `docs/Security/SECRETS_POLICY.md`, enforce `infisical`/GitHub Environments, and add scanning gates.  
  Acceptance: Policy exists; scans enabled; no secrets in repo.

2.3 Observability completeness
- [ ] Extend `monitoring/alerts.yml` with:
  - RTB p95 breach — `auction_latency_seconds` (warn/crit).  
  - Adapter timeout spikes — `rtb_adapter_timeouts_total` (rate 5m/1h).  
  - ClickHouse write failure alerts (ingest/tracking writers).  
  - Queue backlog growth (BullMQ depth/age derivative).  
  - HTTP error budget burn (short 5m / long 1h windows).  
  Acceptance: Alerts present with clear `summary`/`description`, labeled, and validated with `promtool`.
- [ ] Synthetic probes: add Console HTML check; parametrize DEV/STAGING/PROD via inputs/vars.  
  Acceptance: `.github/workflows/synthetic-probes.yml` runs nightly; artifacts (responses) uploaded.

---

#### 3) SDKs
3.1 Android (sdk/core/android)
- [ ] Replace placeholder publishing URL `https://maven.pkg.github.com/ORG/REPO` with real `ORG/REPO`, using a Gradle property/env fallback.  
  Acceptance: Publishing succeeds to GitHub Packages with expected `groupId/artifactId`.
- [ ] Enforce size gate thresholds and fail CI on exceed; emit JSON summary artifact.  
  Acceptance: CI fails when AAR exceeds threshold; size report attached.
- [ ] Add API surface checks (binary compatibility) in CI.  
  Acceptance: Breaking public API changes are detected in PRs.
- [ ] CI: Add Dokka HTML generation and upload artifact for review.  
  Acceptance: Dokka HTML artifact visible on CI runs.
- [ ] Optional: Sample app/instrumentation tests to exercise StrictMode and networking policies.  
  Acceptance: Smoke runs succeed; helps catch runtime regressions.

3.2 iOS (sdk/core/ios)
- [ ] Decide distribution: SPM‑only vs. XCFramework artifacts.
  - If XCFramework: add CI job to archive and attach to GitHub Releases on tag.  
  Acceptance: Chosen policy documented; artifacts published accordingly.
- [ ] Add DocC documentation generation and CI artifact upload.
  Acceptance: Documentation artifact available per build.
- [ ] Expand unit tests: networking, protobuf parsing, crypto workflows (use custom URLProtocol stubs).  
  Acceptance: Tests cover key code paths; CI green.
- [ ] Add semantic versioning gates/breaking‑change checks where feasible.  
  Acceptance: PRs detect source/API breaks.

3.3 Unity (sdk/core/unity, Packages/com.rivalapexmediation.sdk)
- [ ] Verify Unity `package.json` metadata (name, version, displayName, dependencies, samples).  
  Acceptance: UPM manifests validate; Unity Package Manager imports.
- [ ] Ensure Runtime/Editor split, asmdefs, and platform constraints.  
  Acceptance: Assemblies compile across targeted platforms.
- [ ] CI matrix via `game-ci` across LTS versions; cache `Library/` and `Packages/`.  
  Acceptance: CI runs tests/build across matrix reliably.
- [ ] Package UPM tarball and attach to Releases on tag; optional size gate.  
  Acceptance: Release assets include UPM package.

3.4 CTV (sdk/ctv)
- [ ] Android TV: mirror Android core validations (min/target SDK, R8, size check).  
  Acceptance: CI enforces; artifacts published.
- [ ] tvOS: document supported OS/devices; add size reporting if meaningful.  
  Acceptance: CI artifacts and docs updated.

---

#### 4) Backend (backend/)
- [ ] Environment schema validation at startup (e.g., `zod`) and `backend/.env.sample` (no secrets).  
  Acceptance: Missing/malformed envs fail fast with helpful errors.
- [ ] Structured logging with request ID (`x-request-id`), tenant/user fields; propagate IDs to outbound calls.  
  Acceptance: Logs correlate across services; IDs visible in traces/logs.
- [ ] Confirm Express `trust proxy` and TLS termination docs; re‑check rate limit/CSRF behavior behind proxy.  
  Acceptance: Config documented; security middlewares effective in prod topology.
- [ ] Expand integration tests: billing webhook signature validation, transparency metrics, ClickHouse write/rollback.  
  Acceptance: Tests green; increased coverage of critical flows.
- [ ] Container hardening: non‑root user, minimal base (distroless/UBI), health endpoints; add probes in k8s manifests.  
  Acceptance: Image passes scans; probes wired.
- [ ] RED metrics by route via histograms (consider exemplars/OTEL).  
  Acceptance: p50/p95/p99 per route visible in Grafana.

---

#### 5) Console (console/)
- [ ] Enforce ESLint rules for a11y/performance; ensure React strict mode and RSC patterns where applicable.  
  Acceptance: Lint passes; PRs blocked on violations.
- [ ] Integrate Playwright e2e smoke in CI; upload screenshots/videos on failure.  
  Acceptance: Failing e2e produce artifacts for debugging.
- [ ] Add bundle size budget checks with CI report.  
  Acceptance: CI warns/fails when size budgets are exceeded.
- [ ] Ensure MSW is configured in Jest for network stubbing.  
  Acceptance: Unit tests deterministic and fast.

---

#### 6) Documentation and runbooks
6.1 CI/Release guide
- [ ] Add `docs/CI_RELEASE_GUIDE.md` describing unified CI, release, publishing, and rollback; include environment secrets and permissions.
  Acceptance: Developers can perform releases end‑to‑end with the guide.

6.2 Monitoring docs
- [ ] Ensure `docs/Monitoring/GRAFANA_DASHBOARDS.md` includes dashboard screenshots and PromQL snippets.  
  Acceptance: Docs are sufficient to recreate dashboards.
- [ ] Update `docs/Monitoring/ALERTS.md` with "simulate to fire" procedures and runbooks; link from alert annotations.  
  Acceptance: On‑call can acknowledge/triage with documentation links.

6.3 Checklist hygiene
- [ ] Fix inconsistency in `docs/Internal/Development/DEVELOPMENT_TODO_CHECKLIST.md` regarding Unity TCF parsing status.  
  Acceptance: Summary matches detailed sections; no conflicting statuses.

---

#### 7) Monitoring stack enhancements (monitoring/)
- [ ] Add RTB, Tracking/Ingest, DB/Queue dashboards if any are missing, matching metrics listed in `monitoring/README.md`.  
  Acceptance: Dashboards import successfully; panels show data.
- [ ] Add recording rules for common aggregates used in alerts/dashboards.  
  Acceptance: Simpler, more performant PromQL in dashboards/alerts.
- [ ] Validate and lint rules with `promtool`; add a CI step.  
  Acceptance: PRs cannot merge with invalid rules.

---

#### 8) ML and services/inference-ml
- [ ] Enforce deterministic training: seed control, config files, environment capture; lock dependencies (`requirements.txt`/`pyproject.lock`).  
  Acceptance: Reproducible runs on target hardware.
- [ ] Export validations: compare ONNX/TorchScript metrics vs training metrics.  
  Acceptance: CI fails on unacceptable regression deltas.
- [ ] Inference service: CPU-optimized model (quantized), RED metrics, readiness/liveness, Helm chart with autoscaling by p95/QPS.  
  Acceptance: Deployed service meets latency SLOs; metrics exposed.
- [ ] Nightly k6 smoke vs staging; upload JSON artifacts.  
  Acceptance: Nightly artifacts available; threshold regressions alert.

---

#### 9) Quality and scripts
- [ ] Schedule chaos and load tests (nightly) with thresholds and Slack notifications.  
  Acceptance: Nightlies run; regressions notify.
- [ ] Ensure scripts are idempotent and pass shellcheck; add `scripts/README.md` usage and safety notes.  
  Acceptance: Script QA automated in CI; docs available.

---

#### 10) Prioritization and milestones
Priority A (Week 1–2):
- CI/CD consolidation (2.1), Android publishing fix (3.1), alert extensions + synthetic probes (2.3), security image scans + SBOMs (2.2), CI/Release docs (6.1).

Priority B (Week 3–4):
- API compatibility checks (Android/iOS), iOS distribution decision and artifacts (3.2), Unity CI matrix and UPM packaging (3.3), backend env schema/logging/probes (4), console e2e + bundle budgets (5), monitoring recording rules and promtool CI (7).

Priority C (Week 5+):
- ML pipeline/export validations and inference service hardening (8), chaos/load nightly orchestration (9), extended dashboard screenshots/runbooks (6.2), infra pre‑commit/validate (Infrastructure section).

---

#### 11) Acceptance criteria (global)
- Single unified CI gate; deterministic builds; consolidated SDK release process documented and exercised by a tag release.
- Android/iOS/Unity/CTV SDK lanes produce artifacts; size and API gates enforced; Android publishes to GitHub Packages; iOS policy implemented.
- Monitoring alerts extended and validated; synthetic probes cover API and Console; dashboards complete and documented.
- Security scanning: CodeQL + Trivy (FS + images) with SBOMs; policy thresholds enforced.
- Backend/Console: tests green; critical integration flows covered; containers hardened; probes in place.

---

#### 12) Risks and mitigations
- macOS runners availability for iOS/tvOS lanes — Mitigation: queue friendly windows, use `runs-on: macos-13` with retry.
- Flaky external dependencies in CI (Stripe/GCS/S3) — Mitigation: use mocks/stubs for unit/integration; reserve live tests for staging smoke.
- Alert noise after expansion — Mitigation: start with `warning` thresholds; iterate with silences; recording rules for stability.
- Publishing credentials/config drift — Mitigation: environment secrets, double‑checked doc, dry‑run releases in staging.

---

#### 13) Traceability matrix (files to touch)
- Workflows: `.github/workflows/ci-all.yml`, `release-sdks.yml`, `sdk-release.yml`, `synthetic-probes.yml`, `security-trivy.yml`.
- Android SDK: `sdk/core/android/build.gradle`, Gradle props for publishing URL and Dokka task wiring.
- iOS SDK: `sdk/core/ios/Package.swift`, CI job for XCFramework/DocC.
- Unity: `Packages/com.rivalapexmediation.sdk/package.json`, `sdk/core/unity/**`, `unity-sdk.yml`.
- Backend: env schema init, logging middleware, Dockerfile, k8s manifests/probes, tests.
- Console: ESLint config, Playwright config, bundle analyzer.
- Monitoring: `monitoring/alerts.yml`, new dashboard JSONs under `monitoring/grafana/`, `prometheus.yml` recording rules.
- Docs: `docs/CI_RELEASE_GUIDE.md`, `docs/Monitoring/GRAFANA_DASHBOARDS.md`, `docs/Monitoring/ALERTS.md`, `docs/Security/SECRETS_POLICY.md`, checklist updates.

---

#### 14) Next actions (owner to assign)
1) CI/CD consolidation + docs  
2) Android publishing URL fix + Dokka artifact  
3) Monitoring alert extensions + synthetic probes  
4) Security image scans + SBOMs  
5) iOS distribution policy decision and CI implementation  
6) Unity CI matrix and packaging  
7) Backend env schema/logging/probes  
8) Console e2e + bundle budgets  

Upon approval, create tracking issues for each checklist item and execute per milestones above.
