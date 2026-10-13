# Adversarial Dogfooding & 100% Coverage — Master Design

Status: DESIGN (nothing has been executed; all artifacts in this change are declarative configs, tools, and specs).
Owner: @sbelakho2

---

## 0. Definitions and non-negotiable policy

### 0.1 What "100% of repo" means

The repo contains executable code in 11 languages (TS/JS, Python, Go, Rust, Kotlin, Swift, C#, Dart, Bash, SQL, YAML-as-code) plus tests and docs. "100% coverage" is enforced on four metrics:

- **L**: lines, **B**: branches, **F**: functions, **S**: statements (where the toolchain supports all four; tool-specific substitutions are listed per domain in §3).

Every first-party file must end in exactly one of three states recorded in the generated manifest `coverage-manifest.json`:

1. `owned` — claimed by a test suite (or suite-cluster) that must cover it at 100% (L/B/F/S).
2. `excluded` — carries a machine-checkable reason code (§0.3). Exclusions are per-file, never per-directory-glob-without-reason, and each one is audited by the dogfood-audit pipeline.
3. `contract` — non-executable or declarative artifact (SQL, IaC, OpenAPI, monitoring rules, docs) validated by an adversarial contract suite (§5.9) instead of line coverage. Contract suites have their own hard gates; there is no "unvalidated" state.

Rule: **there is no fourth state.** A file that is neither owned, excluded, nor contract-classified makes the dogfood-audit pipeline red (`orphan`).

### 0.2 The "correct expectations always" rule

Every concrete expectation written anywhere in this design (and in every suite implemented from it) MUST carry:

- an **anchor**: `file:line` (or `file` where whole-file behavior is the subject) of the source of truth in this repo, and
- a **behavior-proof note** stating how the expected behavior was established (read of the code, existing passing test, or — where the code's behavior is currently undefined/unknown — the literal marker `AUDIT-REQUIRED`, which makes phase-0 behavior auditing a blocking task before that expectation may be implemented).

Expectations marked `AUDIT-REQUIRED` are decision points, not assertions: the implementer must first record the observed behavior (a failing test that documents reality), then decide whether the behavior is correct (keep expectation) or a bug (fix code, then keep expectation). **An expectation may never be weakened to make a test pass** — this is enforced by the meta-suite (§6.2, M-4 mutation sampling and M-5 expectation-drift diff).

### 0.3 Exclusion reason codes (closed set)

| Code | Meaning | Audited how |
|---|---|---|
| `VENDOR` | third-party code vendored in-repo | provenance header check in meta-suite |
| `GENERATED` | machine-generated from an in-repo generator | generator listed in manifest meta |
| `BUILD_ARTIFACT` | compiled/derived output (dist, .next, coverage) | path-pattern check |
| `SAMPLE_APP` | demo/sample apps: validated by build+smoke contract, not line coverage | contract suite S-SAMPLE |
| `DATA_ARTIFACT` | datasets/models/binary blobs | not code; manifest records only |
| `NON_EXECUTABLE` | images, fonts, logs, editor config | path-pattern check |

Docs (`*.md`) are NOT excluded: they form the `docs` domain with a link/reference contract suite (§5.10).

### 0.4 Enforcement principles ("always")

1. **Ratchet-only**: coverage floors live in `tools/coverage/coverage-floor.json` and may only be raised. Lowering a floor (or deleting a floor file) fails the dogfood-audit pipeline.
2. **No bypasses**: `# pragma: no cover`, `istanbul ignore`, `c8 ignore`, `@CoverageIgnore`, jest `coveragePathIgnorePatterns` additions, `describe.skip`/`it.skip` without a manifest-registered reason, and `|| true` on test/coverage commands are all banned. The existing 21 `# pragma: no cover` markers in ML/services must be deleted or converted into real tests (§3.8). The existing `|| true` soft-fails in `ci-all.yml` are inventoried in §7.3.
3. **Skipped-but-never-run is a bug**: the backend's jest `testMatch` currently executes only 40 of 197 `*.test.ts` files (anchor `backend/jest.config.cjs` — `testMatch: ['<rootDir>/tests/**/*.test.ts']`; 139 files under `src/**/__tests__/`, 15 under `scripts/__tests__/`, 2 under `routes/__tests__/`, 1 under `services/sales-automation/tests/` are silently unmatched). Phase 0 MANDATES consolidating testMatch; the coverage config in this change already does it.
4. **Gates fail the build, not a report**: every percentage below is enforced via `--fail-under`-style mechanisms inside Woodpecker steps, never via "upload and look at it later".
5. **Everything in this design is itself dogfooded**: the manifest builder, gates, and meta-tests are covered by `node --test` suites with their own 100% gate (§6.4).

---

## 1. Current-state audit (facts this design is built on)

Verbatim findings from the 2026-10-13 repo audit (anchors are real paths):

| Area | Tests today | Coverage tooling today | Gate today |
|---|---|---|---|
| backend (TS, ~117k LOC src) | 40 executed of 197 test files | `coverageProvider: 'v8'` only (`backend/jest.config.cjs`) | none |
| backend Go (`auction`, `config`, `payments`, `router`) | 21 `*_test.go` in auction; **0** in the other three modules | none | only dead `ci.yml` ran them |
| console (Next.js) | 44 jest + 27 Playwright | none (no `coverageThreshold`) | none |
| website (Next.js) | 1 jest (TS twin stub dead) + 7 Playwright | none | none |
| control-plane (Express) | 11 jest | none; no coverage script | none |
| sdk/web | 15 jest | `collectCoverage: true` | none |
| sdk/react-native | **0** tests | none | none |
| sdk/core/android + sdk/ctv/android-tv (Kotlin) | 48 + 7 unit test files | **no jacoco/kover** | size gates only |
| sdk/core/ios + sdk/ctv/tvos (Swift) | 21 + 3 XCTest | llvm-cov export only in dead `ci.yml` | none |
| sdk/core/unity (C#) | 13 xunit files that **self-skip** without `UNITY_RUNNER` (anchor `sdk/core/unity/ApexMediation.Tests.csproj`) | Unity TF coverage in a workflow whose trigger path `Packages/...` does not exist | none |
| sdk/flutter | **0** tests | none | none |
| ML (Python) | 15 test files, 33 `def test_` | none; **19 `# pragma: no cover` markers** | none |
| services/fraud-inference + inference-ml (Python) | 8 + 6 pytest fns | 1 `pragma: no cover` each | none |
| services/autopilot (TS) | **0** tests | none | none |
| services/mail-server (Rust, 8 crates) | 6 inline `#[test]` | none | not run in any CI |
| edge-worker (TS) | **0** tests, no test script | none | none |
| quality (Go integration, infra jest, k6) | Go harness + 3 jest specs | none | Go run in `ci-all.yml`; infra specs run nowhere |
| tools/*.js, scripts/*.js (10+ js) | 0 | none | none |
| scripts/**.sh (64 sh) | 0 | none (shellcheck only, no coverage) | none |
| SQL migrations (109 files) | apply-verify only (`backend/scripts/verifyMigrations.js`) | n/a | `ci-all.yml` runs verify only |
| monitoring / infrastructure / helm / terraform | 0 | `promtool check rules` only; no `promtool test rules` (the referenced `alerts.test.yml` does not exist) | rules parse only |
| CI | GitHub Actions only; **Woodpecker absent repo-wide** (0 references) | codecov upload only in dead `ci.yml` | **no coverage threshold anywhere** |

Known broken references the meta-suite must catch and keep caught: `quality/chaos/` referenced by root `test:chaos` (missing), `packages/web-sdk` referenced by `web-sdk.yml`/`kill-switch-smoke-test.yml` (missing), `sdks/*` referenced by `sdk-release.yml`, `compatibility-testing.yml`, CODEOWNERS (missing), `docs/Internal/Development/CI_CONSOLIDATION.md` referenced by `ci.yml` header (missing), `quality/lighthouse/alerts.test.yml`-style promtool reference to nonexistent `monitoring/alerts.test.yml`.

---

## 2. Architecture of the gate stack

```
                 .woodpecker/*.yml (12 pipelines)
  backend ─ frontend ─ node-services ─ go ─ sdk-* ─ python ─ rust ─
  scripts-shell ─ contract ─ dogfood-audit ─────────► coverage-gate
        │                │                │                │
        ▼                ▼                ▼                ▼
  per-domain 100% gates   tools/coverage/ci-reference-integrity   manifest
  (jest/kover/xccov/… )   orphan audit, meta-tests, floors   aggregate verdict
```

- `tools/coverage/build-coverage-manifest.mjs` walks the repo and emits `coverage-manifest.json` (path → domain → gate → suite/status). It is the single definition of "100% of repo".
- `tools/coverage/coverage-gate.mjs` parses per-workspace coverage artifacts (jest `coverage-summary.json`, lcov, `go tool cover -func`, xccov JSON) and fails unless every owned file is claimed and every domain is at its floor (floor=100 at final phase).
- `tools/coverage/ci-reference-integrity.mjs` verifies every path referenced by CI (`working-directory:`, path filters, file uploads) exists — this is the dogfooding guard against the §1 broken-reference class.
- `infrastructure/woodpecker/` contains the self-hosted server/agent design (§7).

---

## 3. Coverage mechanism per domain (exact tooling)

Each domain lists: gate command (run inside the named Woodpecker pipeline), artifact shape, and the four-metric enforcement mechanism.

### 3.1 backend (TS) — pipeline `backend-coverage.yml`

- New additive config `backend/jest.coverage.config.cjs` (created in this change) imports `jest.config.cjs`, then:
  - `collectCoverageFrom`: `['src/**/*.ts', 'routes/**/*.ts', 'services/**/*.ts', 'scripts/**/*.ts', '!**/*.d.ts', '!src/openapi/**']` (mirrors tsconfig `include`; `src/smtp-server.ts` stays IN — it is an entry point, covered via its own suite).
  - `testMatch` consolidation: `tests/**/*.test.ts`, `src/**/__tests__/**/*.test.ts`, `scripts/__tests__/**/*.test.ts`, `routes/__tests__/**/*.test.ts` (fixes §0.4-3).
  - `coverageThreshold.global` = `{branches: 100, functions: 100, lines: 100, statements: 100}`.
  - `coverageReporters` = `['json-summary', 'lcov', 'text']`.
- Gate command: `npx jest -c jest.coverage.config.cjs --runInBand` with `FORCE_DB_SETUP=true`, `RUN_MIGRATIONS_IN_TEST=true`, `SKIP_DB_SETUP=false`, Postgres+Redis services (same env matrix as `ci-all.yml` job `backend` — anchors `.github/workflows/ci-all.yml` steps "Unit tests"/"Verify migrations").
- SQL migration contract runs in the same pipeline (§5.9.1).
- `src/types/*.d.ts` and `src/openapi/billing.yaml` are `contract`-classified (type-shape and schema suites), excluded from line coverage by manifest, not by jest ignore patterns.

### 3.2 console / website / control-plane — pipeline `frontend-coverage.yml`

- `console/jest.coverage.config.js` (additive): extends `jest.config.js`; `collectCoverageFrom: ['src/**/*.{ts,tsx}', '!src/**/*.d.ts', '!src/tests/**']`; threshold 100; `coverageReporters: ['json-summary','lcov']`. Server Components (RSC `page.tsx`) are covered by rendering unit suites under jsdom using the existing babel-jest transform; the design forbids excluding any `src` file from coverage.
- `website/jest.coverage.config.cjs` (additive): extends `jest.config.cjs`, ADDS a babel-jest TS transform (`presets: [['next/babel']]` — same transform chain as console) and widens `testMatch` to `src/**/*.test.{js,ts}`; the dead `.ts` twin of `security.headers` (anchor `website/src/__tests__/security.headers.test.ts`, which is a 1-line stub) is either deleted or implemented — keeping an unrunnable stub is banned by the meta-suite.
- `control-plane/jest.coverage.config.js` (additive): extends `jest.config.js`; `collectCoverageFrom: ['src/**/*.ts', '!src/**/*.d.ts']`; threshold 100. The 3,496-line `src/index.ts` monolith (anchor `control-plane/src/index.ts`) is decomposed-by-tests: route/registration lines via supertest app-assembly suites, listener bootstrap via `listen`-mocked suites.
- Playwright e2e specs are `test-code` (meta domain §6), not part of line coverage.

### 3.3 Node libs & tools — pipeline `node-services-coverage.yml`

- `edge-worker`: new `edge-worker/jest.coverage.config.cjs` (created here) + `test:cov` script (additive in `edge-worker/package.json`); `collectCoverageFrom: ['src/**/*.ts']`; threshold 100. The playwright-based scraper service is covered with HTTP-mock suites (Playwright lib calls mocked at module boundary).
- `services/autopilot`: new `services/autopilot/jest.coverage.config.js`; threshold 100.
- `sdk/web`: `sdk/web/jest.coverage.config.cjs` (additive) extends `jest.config.cjs`, sets `coverageThreshold.global` 100/100/100/100 (it already collects coverage, anchor `sdk/web/jest.config.cjs`).
- `sdk/react-native`: new `sdk/react-native/jest.config.cjs` + coverage config; jsdom; `BannerAd.tsx` covered with @react-native mock harness.
- `tools/*.js`, `quality/lint/*.js`, `quality/tools/*.js`, `scripts/**/*.js`, `console/scripts/*.js`: new `tools/package.json` (workspace `tools/*` already declared in root) providing `c8`-wrapped `node --test` harness over per-file suites; c8 `--100` flag + `--all` (fail when a loaded file is uncovered) and a check that every classified JS file is `import`ed by some suite (orphan-JS check in the manifest).

### 3.4 Go — pipeline `go-coverage.yml`

For `backend/auction`, `backend/config`, `backend/payments`, `backend/router`, `quality/integration`:

```sh
go test -race -covermode=atomic -coverprofile=cover.out ./...
go tool cover -func=cover.out > cover.func
tools/coverage/go-coverage-gate.sh cover.func 100   # fails unless EVERY func = 100.0% and total = 100.0%
```

The three currently untested modules (`config`, `payments`, `router`) get suites from the manifest; `quality/integration` is a test-harness file, so it is `test-code`+covered (dogfooding: the harness itself must be exercised 100% via its own sub-cut suites).

### 3.5 Android (Kotlin) — pipeline `sdk-android-coverage.yml`

- Add `org.jetbrains.kotlinx.kover` plugin + verify rule (minValue 100 on LINE and BRANCH counters) to `sdk/core/android/build.gradle` and `sdk/ctv/android-tv/build.gradle` (diffs included in this change).
- Gate: `./gradlew testDebugUnitTest koverXmlReport koverVerify`. Robolectric (already a test dep) covers Android-framework-coupled classes; pure-Kotlin classes get JVM suites. The 15 third-party adapter stubs under `com/rivalapexmediation/adapter/*` are first-party code: each is owned (error/degradation paths are the adversarial surface), not excluded.
- Instrumented tests (`src/androidTest`, samples): phase-3 optional emulator pipeline; until then they are `test-code` items whose non-execution in CI is recorded (meta-suite asserts the inventory, not coverage).

### 3.6 Apple (Swift) — pipeline `sdk-apple-coverage.yml` (darwin runner, local backend)

```sh
swift test --enable-code-coverage --parallel
xcrun llvm-cov export -format=lcov .build/debug/<PackageTests binary> \
  -instr-profile .build/debug/codecov/default.profdata > coverage.lcov
node tools/coverage/lcov-gate.mjs coverage.lcov 100
```

(The llvm-cov export reuses the approach already proven in the legacy `ci.yml` job
`sdk-ios-test`. The gate fails unless every first-party file reaches 100% lines;
region/branch parity is a documented phase-3 upgrade via `--require-branches` once
llvm-cov region records are wired into the lcov adapter.) Applies to `sdk/core/ios`
and `sdk/ctv/tvos`; requires a darwin Woodpecker agent using the local backend.

### 3.7 Unity (C#) — pipeline `sdk-unity-coverage.yml` (two-tier)

- Tier 1 (no Unity license required): `dotnet test` with `UNITY_RUNNER=true` on the net8 xunit project + `coverlet.collector` threshold 100 (all four metrics) — this covers everything compilable outside the Unity editor; the csproj's self-skip behavior (anchor `sdk/core/unity/ApexMediation.Tests.csproj`) is converted: `UNITY_RUNNER=true` makes `IsTestProject` true, enabling real execution.
- Tier 2 (licensed Unity runner image): Unity Test Framework coverage with `assemblyFilters:+ApexMediation`, XML result, gated to 100 by the lcov/xml adapter. Files only compilable inside Unity (`Platforms/Android`+`Platforms/iOS` bridges calling `UnityEngine`) are owned by tier 2; while tier 2 is not yet wired, they are `planned` — the ratchet makes this visible, never silent.

### 3.8 Python — pipeline `python-coverage.yml`

- ML: add to `ML/pyproject.toml` (done in this change):

```toml
[tool.coverage.run]
source = ["src/ml_pipelines", "scripts"]
branch = true

[tool.coverage.report]
fail_under = 100
show_missing = true
# NOTE: exclude_lines intentionally NOT set; the default "# pragma: no cover"
# exclusion is neutralized by deleting every pragma marker (see below).
exclude_also = []
```

- Gate: `pytest ML/tests ML/scripts/tests --cov --cov-branch --cov-report=lcov --cov-fail-under=100` (pytest-cov honors `fail_under=100`).
- **Pragma elimination**: the design REQUIRES deleting all `# pragma: no cover` markers (19 in `ML/**`, 1 in `services/fraud-inference/main.py:33`, 1 in `services/inference-ml/main.py:24` — anchored) and writing real tests for those lines; the dogfood-audit greps for any new marker and fails. Where a marker guarded genuinely unreachable code (e.g. defensive `except` around `sys.exit`), the code is restructured to be injectable and tested.
- services/fraud-inference, inference-ml: `pytest tests --cov=main --cov-branch --cov-report=lcov --cov-fail-under=100` (FastAPI app exercised via TestClient + adversarial payloads; prom-client registry reset between tests to avoid the cross-test coupling that forced `pragma` markers).

### 3.9 Rust — pipeline `rust-coverage.yml`

```sh
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info --fail-under-lines 100
```

`cargo-llvm-cov` pinned in the pipeline image; branch-level enforcement via the lcov adapter (`tools/coverage/lcov-gate.mjs`) requiring BRH==BRF on every first-party `SF` record (the proto-generated files under `mail-proto` are `GENERATED`-excluded with the generator anchor `proto/mail.proto`).

### 3.10 Flutter — pipeline `sdk-flutter-coverage.yml`

`flutter test --coverage` + `tools/coverage/lcov-gate.mjs coverage/lcov.info 100` (lines; Flutter's lcov lacks branch records — the gate accepts absence of BRH/BRF but fails below 100 lines; branch parity is a documented phase-3 upgrade via `coverage` pkg `branch: true`).

### 3.11 Shell — pipeline `scripts-shell-coverage.yml`

- `bats-core` suites per script group + `kcov` line/branch coverage: `kcov --bash-dont-parse-funcs --bash-parse-files-in-dir=scripts ./cov scripts/<group>.sh` … aggregate via lcov-gate at 100 (lines). Non-interactive behavior is achieved by extracting pure logic into `scripts/lib/*.sh` sourced by the entry scripts (mandated refactor for the 12 largest scripts, e.g. `scripts/deploy-all.sh`); destructive scripts (deploy, ssh, PITL) run only in `--dry-run` mode under test (each entry script must honor a `DOGFOOD_DRY_RUN=1` env that short-circuits side effects — mandated invariant, tested by a contract suite that greps for side-effecting commands outside the dry-run guard).
- `set -euo pipefail` presence is a contract check per script (`shell-gate`).

### 3.12 What is intentionally NOT line-covered

- `src/types/*.d.ts`, protobuf-`GENERATED` code, `docs/**` (contract via §5.10), `data/`, `models/` (DATA_ARTIFACT), `sdk/samples/**` + `sdk/adapter-samples/**` + `Test Apps/**` (SAMPLE_APP: build+smoke contract S-SAMPLE), lockfiles (BUILD_ARTIFACT), `.junie/`, images. Everything else is owned.

---

## 4. Adversarial dogfooding strategy (the "adversarial" layer)

Dogfooding = the repo tests itself with its own machinery, and every layer above is attacked by the layers below:

| Layer | Adversarial technique | Tooling (first-party or already-in-repo) |
|---|---|---|
| Unit | fault injection at boundaries (clock, random, fs, net, db), property-based tests | jest manual mocks (`backend/__mocks__/pg.ts` pattern), fast-check (TS), hypothesis (Python), jqwik (Kotlin, test dep scope only) |
| Integration | real Postgres/Redis services, schema drift attacks, migration up/down round-trips with checksum tampering | `backend/tests/helpers/testDatabase.ts` pattern, `runMigrationsV2.js` |
| E2E | piercing/chaos/soak (already exist for SSAI — extended to billing/RTB) | `backend/tests/ssai/e2e/{piercing,chaos,soak}` pattern |
| Meta | tests-about-tests: skip-ban, `|| true` ban, expectation-drift diff, CI-reference integrity, pragma ban, floor ratchet | `tools/coverage/*` + `quality/meta` suites |
| Mutation | sample-based mutation testing to prove adversarial tests kill mutants (expectation correctness oracle) | StrykerJS (backend/console/control-plane, sampled), mutmut (ML) — phase 3, floor 60% initial, ratcheting |
| Perf/SLO contracts | k6 thresholds asserted as code, not docs | `quality/load-tests/*.js` thresholds (p95<120ms auction, <50ms tracking, <50ms fraud p95) pinned in contract tests |
| IaC/config | promtool `test rules` (new `monitoring/alerts.test.yml`), `helm template`+kubeconform, `terraform validate`, `nginx -t`, `caddy validate`, `docker compose config` | contract pipeline |

Adversarial axes applied to every domain: malformed input (fuzz/property), resource exhaustion (bombs, unbounded growth), race/ordering (concurrency, retries, idempotency), authority (authn/z bypass, privilege escalation, tenant isolation), time (skew, DST, expiry), observability gaps (silent failure must still emit metrics/alerts), and state corruption (crash-between-write, replay, double-spend).

---

## 5. Adversarial expectation catalog

Format: every row is given/when/then with an anchor. `A#` ids are stable and referenced by suite names. Rows marked **AUDIT-REQUIRED** require phase-0 behavior recording before implementation (§0.2). This catalog covers the crown-jewel cases; every remaining owned file inherits its domain's axes and MUST appear in its suite's case list (the manifest ties file→suite; the suite case list ties case→file).

### 5.1 Backend auth/session (suite `S-BACKEND-AUTH`)

| id | given | when | then (exact) | anchor |
|---|---|---|---|---|
| A1 | valid access JWT, `alg=HS256` | request with `alg=none` re-signed token | 401; no token accepted; error code `INVALID_TOKEN` | `backend/src/middleware/auth.ts` AUDIT-REQUIRED (exact error payload) |
| A2 | refresh token rotated once | replay of the pre-rotation refresh token | 401 AND stored family revoked (reuse detection) | `backend/src/repositories/refreshToken.ts` AUDIT-REQUIRED |
| A3 | trust-proxy enabled | `X-Forwarded-For: 1.2.3.4` chain spoofing | limiter keys on the socket peer, not header (rate limits cannot be bypassed by header churn) | `backend/src/middleware/rateLimiting.ts` AUDIT-REQUIRED |
| A4 | valid session | CSRF double-submit with mismatched cookie/header | 403 | `backend/src/middleware/csrf.ts` AUDIT-REQUIRED |
| A5 | api key with scopes | scoped endpoint called with key lacking scope | 403; usage row still recorded (metering cannot be evaded by failure) | `backend/src/middleware/apiKeyAuth.ts` + `repositories/apiKeyUsage.ts` AUDIT-REQUIRED |
| A6 | expired 2FA challenge | proof submitted 1s after TTL | 401; attempt counter incremented; lockout after N attempts | `backend/src/controllers/*twofa*` AUDIT-REQUIRED |

### 5.2 Backend billing/money (suite `S-BACKEND-BILLING`)

| id | given | when | then | anchor |
|---|---|---|---|---|
| B1 | usage events for tenant | meter replay (same event id twice) | idempotent: charged once; duplicate dropped, metric `meter_duplicates_total` bumped | `backend/src/services/billing/UsageMetering*` AUDIT-REQUIRED |
| B2 | negative/NaN/huge quantities (`1e308`, `-1`, `NaN`) | metering ingest | rejected by zod schema; no partial write; alert metric | `backend/src/config/env.ts` + billing schemas; AUDIT-REQUIRED (current accept/reject) |
| B3 | Stripe webhook valid sig, `t` 10 min old | process | rejected (Stripe tolerance 5 min) | `backend/src/routes/*billing*` AUDIT-REQUIRED |
| B4 | invoice period boundary | usage at exact `periodEnd` timestamp | attributed to one period only; rounding = banker's rounding to cent, no float accumulation | `services/billing/platformFeeService` (top-level backend/services/billing/) AUDIT-REQUIRED |
| B5 | payout already in `paid` state | duplicate payout request | 409 or no-op; ledger invariant `sum(entries)==0` holds | AUDIT-REQUIRED |

### 5.3 Backend RTB/auction (suite `S-BACKEND-RTB`)

| id | given | when | then | anchor |
|---|---|---|---|---|
| R1 | OpenRTB bidrequest | `imp[]` empty / `bidfloor: -1` / `tmax: 0` | deterministic rejection (400) with no DSP fan-out | `backend/src/services/rtbEngine` AUDIT-REQUIRED |
| R2 | DSP responds after internal deadline | hedge race | winning path returns before deadline; late response discarded and counted in timeout metrics | `backend/src/services/rtbEngine`, Go `backend/auction/internal/bidding` (hedge tests exist: anchor `backend/auction/internal/bidding/*_test.go`) |
| R3 | waterfall with cycling adapters A→B→A | walk | cycle detected, no infinite loop, fallback wins fill | `backend/src/services/waterfallService` AUDIT-REQUIRED |
| R4 | thompson sampling priors | adversarial click-fraud feedback (all clicks on one arm) | distribution stays bounded; no divide-by-zero when impressions=0 | `backend/src/services/thompsonSamplingService` AUDIT-REQUIRED |
| R5 | Go auction p95 window empty | compute p95 | no NaN; documented sentinel | `backend/auction/internal/bidding` AUDIT-REQUIRED |

### 5.4 Backend SSAI (suite `S-BACKEND-SSAI`, extends existing chaos/piercing/variants)

| id | given | when | then | anchor |
|---|---|---|---|---|
| S1 | VAST wrapper | wrapper chain loops (A wraps A) | unwrapping bounded by max depth; error event emitted; no hang | `backend/src/services/ssai/vast*` + existing `tests/ssai/e2e/piercing` pattern |
| S2 | manifest | `#EXT-X-TARGETDURATION` lied (segments longer) | player-safe output or explicit reject; never silent | `backend/src/services/ssai/manifest*` AUDIT-REQUIRED |
| S3 | live session | seek beyond DVR window | bounded response; tracking beacons deduped | `backend/src/services/ssai/session*` AUDIT-REQUIRED |
| S4 | beacon storm | same beacon id x1000 in 1s | deduped by idempotency key; `trackingRateLimiter` engages | `backend/src/middleware/trackingRateLimiter.ts` AUDIT-REQUIRED |
| S5 | DRM init mismatch | audio/video init segments from different keys | hard fail with structured error; no stitched output | existing `tests/ssai/e2e/drm` extension |

### 5.5 Backend queues/schedulers (suite `S-BACKEND-OPS`)

| id | given | when | then | anchor |
|---|---|---|---|---|
| Q1 | BullMQ queue | poison job (non-serializable, throws every retry) | retries bounded by config; dead-lettered; queue depth alert fires | `backend/src/queues/queueManager.ts`, `processors/*` AUDIT-REQUIRED |
| Q2 | cron scheduler | overlapping tick during long previous run | no concurrent double-run (lock or skip semantics), documented which | `backend/src/services/*Scheduler*` (5 schedulers) AUDIT-REQUIRED |
| Q3 | DB pool | all connections held | request path fails fast with 503 + metric, never hangs > pool timeout | `backend/src/utils/postgres.ts` AUDIT-REQUIRED |
| Q4 | redis outage | circuit breaker opens | fail-open vs fail-closed per route class is EXPLICIT and tested; no silent fallback | `backend/src/utils/redisCircuitBreaker.ts` AUDIT-REQUIRED |
| Q5 | env zod schema | missing secret at boot | process refuses to start with named-variable error | `backend/src/config/env.ts` (behavior verified by ci-all env-schema check step) |

### 5.6 Backend migration/SQL (suite `S-BACKEND-SQL`, contract)

| id | given | when | then | anchor |
|---|---|---|---|---|
| M1 | all 109 migrations | up→down→up round-trip | schema identical (dump diff), checksums stable | `backend/scripts/runMigrationsV2.js` + `verifyMigrations.js` |
| M2 | checksum-verified history | tamper with one applied `.up.sql` | verify fails naming the file | `backend/scripts/runMigrationsV2.js` (checksum column) |
| M3 | raw SQL in 31 src files | injection fuzz on every string-interpolated query | parameterized only; grep contract bans template-literal SQL interpolation | `tools/coverage/sql-interpolation-gate.mjs` (new; phase 1) |

### 5.7 Control-plane (suite `S-CP-*`)

| id | given | when | then | anchor |
|---|---|---|---|---|
| C1 | global rate limit 500/15min, sensitive 10/hour | distributed XFF spoofing | counts per real peer; sensitive ops capped | `control-plane/src/index.ts` (limits in source) AUDIT-REQUIRED |
| C2 | CIDR auth | IPv4-mapped IPv6 (`::ffff:10.0.0.1`) and IPv6-formatted CIDR comparisons | canonicalized before match; bypass impossible | `control-plane/src/auth.ts` + existing `tests/auth-cidr.test.ts` extension |
| C3 | inbound SMTP server | CRLF-injection in `MAIL FROM`, bare-LF smuggling, oversized DATA | rejected; parser state machine cannot be desynced | `control-plane/src/services/apexmail/inbound-smtp.ts` AUDIT-REQUIRED |
| C4 | kill switch | toggled while orchestrator mid-flight | in-flight actions complete or abort atomically; events emitted (existing `kill-switch-unified-events.test.ts` extended) | `control-plane/src/services/global-kill-switch.ts` |
| C5 | pdf-generator | invoice with 10^6 line items / unicode bomb | bounded memory+time; error, not OOM | `control-plane/src/services/pdf-generator.ts` AUDIT-REQUIRED |
| C6 | node-cron sales scheduler | clock jump forward 1h (DST) | missed jobs backfilled at most once; no duplicate sends | `control-plane/src/services/sales-automation/sales-scheduler.ts` AUDIT-REQUIRED |
| C7 | hybrid classifier | ONNX edge-worker 500s/hangs | circuit breaker trips; Naive-Bayes-only degradation path scores mail; queue persists | `control-plane/src/services/inbox/hybrid-classifier.ts` + `edge-worker-client.ts` AUDIT-REQUIRED |

### 5.8 SDKs (suites `S-SDK-*`)

| id | given | when | then | anchor |
|---|---|---|---|---|
| K1 | kill switch cached (Android) | flip mid-auction | next request honors switch; in-flight completes; cache TTL honored | existing `*KillSwitch*` tests (anchor `kill-switch-smoke-test.yml`) |
| K2 | UMP consent | callback arrives AFTER ad request fired | request blocked or re-run post-consent; never both; GDPR region never serves unconsented | `sdk/core/android/src/main/kotlin/consent/*` AUDIT-REQUIRED |
| K3 | OMSDK binary absent | ad render | degrade to image beacon path; no crash (existing `OmSdkAbsentTest` extended) | `sdk/ctv/android-tv/src/test/*OmSdkAbsentTest*` |
| K4 | TCF string | malformed/oversized consent string | parser fails closed (no consent), never throws across API boundary | `sdk/ctv/android-tv/.../consent/TCFParser` + tvos `TCFParserTests` extension |
| K5 | Swift strict concurrency | ad request cancelled mid-flight | no detached-task leak; actor state consistent (`-strict-concurrency` build already on) | `sdk/core/ios/Package.swift` (StrictConcurrency) |
| K6 | Web SDK SSAI parser | manifest with duplicate `#EXT-X-DISCONTINUITY`, zero-duration segments | parsed deterministically or rejected; no NaN in timeline | `sdk/web/src/ssai/SSAIManifestParser*` (existing tests extended) |
| K7 | Unity xunit without Unity | `UNITY_RUNNER` unset | Tier-1 gate FAILS the build if `IsTestProject=false` observed (self-skip ban) | `sdk/core/unity/ApexMediation.Tests.csproj` |

### 5.9 Contract domain suites

| id | target | adversarial check | gate |
|---|---|---|---|
| S-SQL | 109 migration files + `tools/sql` | M1/M2/M3 above | `backend-coverage.yml` step |
| S-IAC | monitoring rules, helm, terraform, nginx, caddy, compose | `promtool test rules monitoring/alerts.test.yml` (created in phase 1 with adversarial alert-firing cases), `helm lint && helm template \| kubeconform -strict`, `terraform validate`, `nginx -t`, `caddy validate`, `docker compose config` | `contract-coverage.yml` |
| S-OPENAPI | `backend/src/openapi/billing.yaml` | redocly lint + drift vs baseline + every documented route exists in code (and vice versa) | `contract-coverage.yml` |
| S-CI | `.github/workflows/*.yml` + `.woodpecker/*.yml` | every referenced path exists (`ci-reference-integrity`), no `|| true` outside allowlist, required-checks parity, woodpecker gate set covers all domains | `contract-coverage.yml` + `dogfood-audit.yml` |
| S-SAMPLE | `sdk/samples/**`, `adapter-samples`, `Test Apps` | all sample apps compile (`build` contracts) + smoke boot | `contract-coverage.yml` |
| S-CONTENT | `tools/content-guard.js` | guard fed adversarial provider strings (homoglyphs, split tokens) — guard itself must classify correctly | `contract-coverage.yml` |

### 5.10 Docs domain (suite `S-DOCS`)

Linkcheck (internal anchors + cross-references) + freshness contract: every doc referencing an npm script/Makefile target/env var references something that exists (same engine as `ci-reference-integrity`, applied to `docs/**`). Adversarial case S-DOCS-1: rename a script → docs contract fails.

---

## 6. Meta-adversarial suite (the repo's tests are also attacked)

### 6.1 Invariants enforced by `dogfood-audit.yml`

| id | rule | mechanism |
|---|---|---|
| M-1 | no orphan source files | manifest build + orphan gate |
| M-2 | floors ratchet-only | `coverage-floor.json` diff check (decrease = fail) |
| M-3 | no coverage-ignoring markers/`describe.skip` without registry | grep gate over `src`+`tests` of all workspaces |
| M-4 | no `|| true` on test/coverage commands outside allowlist | YAML grep gate (existing `ci-all.yml` occurrences inventoried in §7.3; each must be converted or explicitly allowlisted with reason) |
| M-5 | expectation-drift: an expectation changed from fail→pass semantics without a linked bug-fix commit fails audit | diff-grep on test files for `toBe`/`toEqual` relaxations (`expect.any`, widened matchers) — sampled by mutation testing in phase 3 |
| M-6 | CI reference integrity (all paths in workflows exist) | `tools/coverage/ci-reference-integrity.mjs` |
| M-7 | every test file contains at least one assertion (kills always-green tests) | meta test scan |
| M-8 | self-coverage: `tools/coverage/*` covered by `node --test` at 100% | `node --test --experimental-test-coverage tools/coverage/test/` + parse gate |

### 6.2 Why mutation testing is the "correct expectations" oracle

A passing suite only proves expectations hold for the written inputs. Mutation sampling (StrykerJS/mutmut) mutates production code and requires the suite to kill mutants; an expectation that does not pin behavior lets mutants survive, which the audit scores and fails below floor. This is the only mechanical proof that expectations are *correct and tight*; hence phase 3 floors at 60% kill rate with ratchet to 80%.

---

## 7. Woodpecker design

### 7.1 Server/runner

Self-hosted design in `infrastructure/woodpecker/` (docker-compose for server+agent on the existing Hetzner app-server pattern; GitHub OAuth app integration; commit-status reporting gives GitHub branch protection the same power as `required-checks.json` — the design mirrors required checks: `coverage-gate`, `dogfood-audit`, `contract-coverage` are the protected-branch contexts).

### 7.2 Pipeline inventory (`.woodpecker/`, 14 pipelines)

| pipeline | covers domains | key services |
|---|---|---|
| `backend-coverage.yml` | backend TS + SQL | postgres:16, redis:7 |
| `frontend-coverage.yml` | console, website, control-plane | none (jsdom/supertest) |
| `node-services-coverage.yml` | edge-worker, autopilot, sdk/web, sdk/react-native, tools JS | none |
| `go-coverage.yml` | 4 backend Go modules + quality Go | postgres (auction integration) |
| `sdk-android-coverage.yml` | Kotlin core + ctv | none (Robolectric) |
| `sdk-apple-coverage.yml` | iOS + tvOS | none (darwin agent) |
| `sdk-unity-coverage.yml` | C# tiers 1+2 | none |
| `sdk-flutter-coverage.yml` | Dart | none |
| `python-coverage.yml` | ML, fraud-inference, inference-ml | postgres (fraud) |
| `rust-coverage.yml` | mail-server | none |
| `scripts-shell-coverage.yml` | 64 sh + tools sh | none (kcov in-image) |
| `contract-coverage.yml` | SQL/IaC/OpenAPI/CI/content/samples/docs | postgres (SQL apply) |
| `dogfood-audit.yml` | manifest, meta M-1..M-8 | none |
| `coverage-gate.yml` | aggregate verdict | none; `depends_on` all above |

All pipelines trigger on `push`+`pull_request` to `main`/`master` with **no path filters** in phase 0 (rationale: path-filtered parents that are skipped must not silently weaken the aggregate gate; filters are a phase-4 optimization behind verified `depends_on`-skip semantics). Secrets: none required for gates; optional `codecov_token` push-only upload step per pipeline (PR events never receive secrets — kept out of gate logic by design).

### 7.3 Existing `|| true` inventory (M-4 backlog, each must be converted or allowlisted with reason)

`ci-all.yml`: `console-a11y-perf` lighthouse, `billing-e2e-smoke` playwright, `website-a11y-perf` lighthouse+analyze, `website-visual-regression`; `nightly-billing-perf.yml` k6. None of these may guard coverage numbers.

### 7.4 Rollout phases

- **Phase 0 (this change)**: Woodpecker infra design + pipelines + manifest tooling + additive coverage configs + catalog + meta gates. Gates ratchet from observed baselines; `coverage-floor.json` seeded so the first run is green but every number is pinned.
- **Phase 1**: implement suites per domain in ratchet order (smallest first: tools/js, shell lib extraction, python services, ML pragmas); floors +10/week until 100.
- **Phase 2**: mutation floors; instrumented-Android emulator lane; Unity tier-2 licensed lane.
- **Phase 3**: branch-parity for Apple (region gate), Flutter branch, path-filter optimization after `depends_on`-skip semantics verified.
- **Completion criterion**: `coverage-gate` green with every domain floor = 100 and manifest orphans = 0, for 14 consecutive days on `main`.

---

## 8. Self-verification protocol for THIS design

This design was produced without executing anything (per mandate). It is checked by four independent rigorous audits, run as read-only subagent reviews:

1. **Woodpecker semantics audit** — every `.woodpecker/*.yml` and `infrastructure/woodpecker/*` checked against Woodpecker 2.x semantics (when/steps/services/secrets/depends_on/plugin settings) and against real repo paths/scripts/env vars.
2. **Completeness audit** — an independent file enumeration (find/ls, NOT the manifest tool itself) diffed against the manifest domain rules to find any unclassified file/type/directory.
3. **Expectation correctness audit** — every anchored expectation in §5 checked against the cited source; mismatches or overclaims reported and fixed.
4. **Mechanism audit** — every gate command/flag/config (jest thresholds, pytest-cov, kover DSL, xccov fields, cargo-llvm-cov flags, go cover parsing, kcov, c8) verified to exist in the pinned tool versions.

Findings are fixed before this design is considered final; the fix log is appended in §9.

---

## 9. Verification fix log

(appended after audits; empty at design time)
