# Adversarial Dogfooding & 100% Coverage — Master Design

Status: design complete; the stack is implemented and verified. The Rust product workspace, the Zola site, the console audit lane, and the dogfood gates all run green locally (§9.5); phase-gated items stay visible in §9.3. The 2026-10-14 all-Rust conversion replaced the multi-language stack with the `apex-tools` gates; §9.7 records the conversion sweep and supersedes the lane inventory in §7.2 and the per-domain tooling in §3.
Owner: @sbelakho2

---

## 0. Definitions and non-negotiable policy

### 0.1 What "100% of repo" means

The repo contains executable code in 11 languages (TS/JS, Python, Go, Rust, Kotlin, Swift, C#, Dart, Bash, SQL, YAML-as-code) plus tests and docs. "100% coverage" is enforced on four metrics:

- `L`: lines, `B`: branches, `F`: functions, `S`: statements (where the toolchain supports all four; tool-specific substitutions are listed per domain in §3).

Every first-party file must end in exactly one of three states recorded in the generated manifest `coverage-manifest.json`:

1. `owned`: claimed by a test suite (or suite-cluster) that must cover it at 100% (L/B/F/S).
2. `excluded`: carries a machine-checkable reason code (§0.3). Exclusions are per-file, never per-directory-glob-without-reason, and each one is audited by the dogfood-audit pipeline.
3. `contract`: non-executable or declarative artifact (SQL, IaC, OpenAPI, monitoring rules, docs) validated by an adversarial contract suite (§5.9) instead of line coverage. Contract suites have their own hard gates; there is no "unvalidated" state.

Rule: **there is no fourth state.** A file that is neither owned, excluded, nor contract-classified makes the dogfood-audit pipeline red (`orphan`).

### 0.2 The "correct expectations always" rule

Every concrete expectation written anywhere in this design (and in every suite implemented from it) MUST carry:

- an **anchor**: `file:line` or `file` for whole-file behavior, naming the source of truth in this repo.

- a **behavior-proof note**: how the expected behavior was established (a code read, an existing passing test, or the literal marker `AUDIT-REQUIRED` when the behavior is unknown, which blocks phase-0 auditing before that expectation may be implemented).

Expectations marked `AUDIT-REQUIRED` are decision points, not assertions: record the observed behavior first (a failing test that documents reality), then decide whether it is correct (keep the expectation) or a bug (repair the code, then keep the expectation). **An expectation may never be weakened to make a test pass** — this is enforced by the meta-suite (§6.2, M-4 mutation sampling and M-5 expectation-drift diff).

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

2. **No bypasses**: `# pragma: no cover`, `istanbul ignore`, `c8 ignore`, `@CoverageIgnore`, jest `coveragePathIgnorePatterns` additions, `describe.skip`/`it.skip` without a manifest-registered reason, and `|| true` on test/coverage commands are all banned. The 20 `# pragma: no cover` markers across 13 files in ML/services must be deleted or converted into real tests (§3.8), and the `|| true` soft-fails are inventoried in §7.3.

3. **Skipped-but-never-run is a bug**: the backend's jest `testMatch` ran only 40 of 197 `*.test.ts` files. The additive `backend/jest.coverage.config.cjs` consolidates `testMatch` to pick up `src/**/__tests__`, `scripts/__tests__`, `routes/__tests__`, and `services/sales-automation/tests`. The Unity tier-1 suite had never compiled at all (see §5.11 U-1) and now runs 25/25 green.

4. **Gates fail the build, not a report**: every percentage below is enforced via `--fail-under`-style mechanisms inside Woodpecker steps, never via "upload and look at it later". Machine-readable floors live in `tools/coverage/coverage-floor.json` and are mirrored to the grep-readable `tools/coverage/floors.txt` (kept in lockstep by `apex-tools sync-floors` plus the dogfood audit; see §9.7 for the all-Rust tooling).

5. **Everything in this design is itself dogfooded**: the gate tooling runs under its own self-coverage gate (§6.1 M-8), and the prose lint plus the stub-ban check gate the files that implement them (§6.1 M-9/M-10).

---

## 1. Current-state audit (facts this design is built on)

Findings from the 2026-10-13 repo audit, refreshed to the current tree (anchors are real paths):

| Area | Tests today | Coverage tooling today | Gate today |
|---|---|---|---|
| backend (TS, ~117k LOC src) | 197 test files | `coverageProvider: 'v8'` plus the additive `jest.coverage.config.cjs` (widened `testMatch`) | `backend-coverage.yml` floor (0, ratcheting) |
| backend Go (`auction`, `config`, `payments`, `router`) | 21 `*_test.go` in auction; 0 in the other three modules | none yet | `go-coverage.yml` floor (0, ratcheting) |
| console (Next.js) | 44 jest + 27 Playwright | additive `jest.coverage.config.js` | `frontend-coverage.yml` floor (0, ratcheting) |
| website (Next.js) | 1 jest suite plus its implemented TS twin + 7 Playwright | additive `jest.coverage.config.cjs` | `frontend-coverage.yml` floor (0, ratcheting) |
| control-plane (Express) | 11 jest | additive `jest.coverage.config.js` | `frontend-coverage.yml` floor (0, ratcheting) |
| sdk/web | 15 jest | `collectCoverage: true` plus `jest.coverage.config.cjs` (threshold 100) | `node-services-coverage.yml` floor (0, ratcheting) |
| sdk/react-native | 0 tests | new `jest.config.cjs` + coverage config | `node-services-coverage.yml` floor (0, ratcheting) |
| sdk/core/android + sdk/ctv/android-tv (Kotlin) | 48 + 7 unit test files | kover 0.8.3 `koverVerify` on line and branch counters | `sdk-android-coverage.yml` floor (0, ratcheting) |
| sdk/core/ios + sdk/ctv/tvos (Swift) | 21 + 3 XCTest | llvm-cov lcov export | `sdk-apple-coverage.yml` floor (0, ratcheting) |
| sdk/core/unity (C#) | tier-1 suite runs 25/25 via `dotnet test`; 47 files at a 31% lcov baseline | coverlet lcov tier-1 | `sdk-unity-coverage.yml` floor (0, ratcheting); tier-2 licensed lane is phase 2 (§3.7) |
| sdk/flutter | 0 tests | none yet | `sdk-flutter-coverage.yml` floor (0, ratcheting) |
| ML (Python) | 15 test files, 33 `def test_` | 18 `# pragma: no cover` markers across 11 files, frozen in the policy baseline | `python-coverage.yml` floor (0, ratcheting); markers are phase-1 deletions (§3.8) |
| services/fraud-inference + inference-ml (Python) | 8 + 6 pytest fns | 1 `pragma: no cover` each | `python-coverage.yml` floor (0, ratcheting) |
| services/autopilot (TS) | 0 tests | new `jest.coverage.config.js` | `node-services-coverage.yml` floor (0, ratcheting) |
| services/mail-server (Rust, 8 crates) | 6 inline `#[test]` | cargo-llvm-cov lcov | `rust-coverage.yml` floor (0, ratcheting) |
| edge-worker (TS) | 0 tests, no test script | new `jest.coverage.config.cjs` + `test:cov` | `node-services-coverage.yml` floor (0, ratcheting) |
| quality (Go integration, infra jest, k6) | Go harness + 3 jest specs | none yet | `go-coverage.yml` + `contract-coverage.yml` |
| tools/**.mjs + quality/scripts JS | 42 `node --test` cases under `tools/` | c8 via `npm run test:cov` | js-tools floor plus M-8 self-coverage |
| scripts/**.sh (64 sh) | 0 bats suites yet | kcov in `scripts-shell-coverage.yml` | shell floor (0, ratcheting) |
| SQL migrations (109 files) | apply-verify only (`backend/scripts/verifyMigrations.js`) | n/a | `backend-coverage.yml` SQL contract step |
| monitoring / infrastructure / helm / terraform | 0 | `promtool check rules` plus `monitoring/alerts.test.yml` | `contract-coverage.yml` S-IAC |
| CI | GitHub Actions plus `.woodpecker/` (all-Rust tree: 6 configs, 5 lanes + the aggregate verdict, §9.7) | Woodpecker gates configured and locally verified (§9.8) | floors fail the build inside each pipeline; `coverage-gate` is the aggregate verdict |

Known broken references the meta-suite must catch and keep caught:

- `quality/chaos/` is referenced by the root `test:chaos` script and is absent.
- `sdks/*` paths in `sdk-release.yml` and `compatibility-testing.yml` are absent; both entries are frozen in `ci-ref-baseline.json` (ratchet: an entry that resolves fails as stale).
- The `ci.yml` header references the absent `docs/Internal/Development/CI_CONSOLIDATION.md`.
- `monitoring/alerts.test.yml` now exists and is exercised by the promtool contract.

---

## 2. Architecture of the gate stack

```
                 .woodpecker/*.yml (all-Rust tree: 5 lanes plus the aggregate verdict, §9.7)
  backend ─ frontend ─ node-services ─ go ─ sdk-* ─ python ─ rust ─ apex-rs ─
  scripts-shell ─ contract ─ dogfood-audit ─ site-perf ─ ui-ux ───► coverage-gate
        │                │                │                │
        ▼                ▼                ▼                ▼
  per-domain 100% gates   contract checks   manifest
  (jest/kover/xccov/… )   orphan audit, meta-tests, floors   aggregate verdict
```

- `tools/coverage/build-coverage-manifest.mjs` walks the repo and emits `coverage-manifest.json` (path → domain → gate → suite/status). It is the single definition of "100% of repo" and currently classifies 2,904 files with 0 unclassified.
- `tools/coverage/coverage-gate.mjs` parses per-workspace coverage artifacts (jest `coverage-summary.json`, lcov, `go tool cover -func`, xccov JSON) and fails unless every owned file is claimed and every domain is at its floor (floor=100 at final phase).
- `tools/coverage/contract-checks.mjs` (subcommand `ci-reference-integrity`) verifies every path referenced by CI (`working-directory:`, path filters, file uploads) exists; this is the dogfooding guard against the §1 broken-reference class.
- `infrastructure/woodpecker/` contains the self-hosted server/agent design (§7).

---

## 3. Coverage mechanism per domain (exact tooling)

Each domain lists: gate command (run inside the named Woodpecker pipeline), artifact shape, and the four-metric enforcement mechanism.

### 3.1 backend (TS) — pipeline `backend-coverage.yml`

- New additive config `backend/jest.coverage.config.cjs` (created in this change) imports `jest.config.cjs`, then:
  - `collectCoverageFrom`: `['src/**/*.ts', 'routes/**/*.ts', 'services/**/*.ts', 'scripts/**/*.ts', '!**/*.d.ts', '!src/openapi/**']` (mirrors tsconfig `include`; `src/smtp-server.ts` stays IN — it is an entry point, covered via its own suite).
  - `testMatch` consolidation: `tests/**/*.test.ts`, `src/**/__tests__/**/*.test.ts`, `scripts/__tests__/**/*.test.ts`, `routes/__tests__/**/*.test.ts` (answers §0.4-3).
  - `coverageThreshold.global` = `{branches: 100, functions: 100, lines: 100, statements: 100}`.
  - `coverageReporters` = `['json-summary', 'lcov', 'text']`.
- Gate command: `npx jest -c jest.coverage.config.cjs --runInBand` with `FORCE_DB_SETUP=true`, `RUN_MIGRATIONS_IN_TEST=true`, `SKIP_DB_SETUP=false`, Postgres+Redis services (same env matrix as `ci-all.yml` job `backend` — anchors `.github/workflows/ci-all.yml` steps "Unit tests"/"Verify migrations").
- SQL migration contract runs in the same pipeline (§5.9.1).
- `src/types/*.d.ts` and `src/openapi/billing.yaml` are `contract`-classified (type-shape and schema suites), excluded from line coverage by manifest, not by jest ignore patterns.

### 3.2 console / website / control-plane — pipeline `frontend-coverage.yml`

- `console/jest.coverage.config.js` (additive): extends `jest.config.js`; `collectCoverageFrom: ['src/**/*.{ts,tsx}', '!src/**/*.d.ts', '!src/tests/**']`; threshold 100; `coverageReporters: ['json-summary','lcov']`. Server Components (RSC `page.tsx`) are covered by rendering unit suites under jsdom using the existing babel-jest transform; the design forbids excluding any `src` file from coverage.
- `website/jest.coverage.config.cjs` (additive): extends `jest.config.cjs`, adds a babel-jest TS transform (`presets: [['next/babel']]`, the same chain as console) and widens `testMatch` to `src/**/*.test.{js,ts}`. The TS twin of `security.headers` (anchor `website/src/__tests__/security.headers.test.ts`) is implemented; an unrunnable stub is banned by the meta-suite.
- `control-plane/jest.coverage.config.js` (additive): extends `jest.config.js`; `collectCoverageFrom: ['src/**/*.ts', '!src/**/*.d.ts']`; a global threshold of 100. The 3,496-line `src/index.ts` monolith (anchor `control-plane/src/index.ts`) is decomposed-by-tests: route/registration lines via supertest app-assembly suites, listener bootstrap via `listen`-mocked suites.
- Playwright e2e specs are `test-code` (meta domain §6), not part of line coverage.

### 3.3 Node libs & tools — pipeline `node-services-coverage.yml`

- `edge-worker`: new `edge-worker/jest.coverage.config.cjs` (created here) + `test:cov` script (additive in `edge-worker/package.json`); `collectCoverageFrom: ['src/**/*.ts']`; threshold 100. The playwright-based scraper service is covered with HTTP-mock suites (Playwright lib calls mocked at module boundary).
- `services/autopilot`: new `services/autopilot/jest.coverage.config.js`; threshold 100.
- `sdk/web`: `sdk/web/jest.coverage.config.cjs` (additive) extends `jest.config.cjs`, sets `coverageThreshold.global` 100/100/100/100 (it already collects coverage, anchor `sdk/web/jest.config.cjs`).
- `sdk/react-native`: new `sdk/react-native/jest.config.cjs` + coverage config; jsdom; `BannerAd.tsx` covered with @react-native mock harness.
- `tools/**/*.mjs` (gate tooling, site gates, ui-ux audit, content guard) plus `quality/lint/*.js`, `quality/tools/*.js`, `scripts/**/*.js`, `console/scripts/*.js`: the `tools` package runs a c8-wrapped `node --test` harness; `npm run test:cov` covers `coverage/*.mjs`, `site/*.mjs`, `ui-ux/*.mjs`, and `content-guard.js`.

### 3.4 Go — pipeline `go-coverage.yml`

For `backend/auction`, `backend/config`, `backend/payments`, `backend/router`, `quality/integration`:

```sh
go test -race -covermode=atomic -coverprofile=cover.out ./...
go tool cover -func=cover.out > cover.func
tools/coverage/go-coverage-gate.sh cover.func 100   # fails unless EVERY func = 100.0% and total = 100.0%
```

The three currently untested modules (`config`, `payments`, `router`) get suites from the manifest; `quality/integration` is a test-harness file, so it is `test-code`+covered (dogfooding: the harness itself must be exercised 100% via its own sub-cut suites).

### 3.5 Android (Kotlin) — pipeline `sdk-android-coverage.yml`

- The `org.jetbrains.kotlinx.kover` plugin (0.8.3) and its verify rule are wired into `sdk/core/android/build.gradle` and `sdk/ctv/android-tv/build.gradle`; `minValue` is read from the ratcheting floor in `floors.txt` (final target 100 on the line and branch counters).
- Gate: `./gradlew testDebugUnitTest koverXmlReport koverVerify`. Robolectric (already a test dep) covers Android-framework-coupled classes; pure-Kotlin classes get JVM suites. The 15 third-party adapter stubs under `com/rivalapexmediation/adapter/*` are first-party code: each is owned (error/degradation paths are the adversarial surface), not excluded.
- Instrumented tests (`src/androidTest`, samples): phase-3 optional emulator pipeline; until then they are `test-code` items whose non-execution in CI is recorded (meta-suite asserts the inventory, not coverage).

### 3.6 Apple (Swift) — pipeline `sdk-apple-coverage.yml` (darwin runner, local backend)

```sh
swift test --enable-code-coverage --parallel
xcrun llvm-cov export -format=lcov .build/debug/<PackageTests binary> \
  -instr-profile .build/debug/codecov/default.profdata > coverage.lcov
node tools/coverage/lcov-gate.mjs coverage.lcov 100
```

The llvm-cov export reuses the approach from the legacy `ci.yml` job `sdk-ios-test`. The gate fails unless every first-party file reaches 100% lines; region and branch parity is a phase-3 upgrade via `--require-branches` once llvm-cov region records feed the lcov adapter. This applies to `sdk/core/ios` and `sdk/ctv/tvos`; it requires a darwin Woodpecker agent using the local backend.

### 3.7 Unity (C#) — pipeline `sdk-unity-coverage.yml` (two-tier)

- Tier 1 (no Unity license; runs green today): the runtime builds as `sdk/core/unity/Runtime/ApexMediation.Runtime.csproj`, a tier-1-only project Unity never reads. `Tests/UnityStubs.cs` shims the plain-.NET UnityEngine surface, and tests run as `dotnet test sdk/core/unity/Tests/ApexMediation.Tests.csproj` under `UNITY_RUNNER=true`, which inverts the csproj self-skip into real execution. Coverlet emits `sdk/core/unity/coverage/lcov.info`; a local run gave 25/25 tests, 47 files, and a 31% line baseline, from which the floor gate ratchets.
- Tier 2 (licensed Unity runner image, phase 2): Unity Test Framework coverage with `assemblyFilters:+ApexMediation` for the editor-coupled surface (`ApexMediationEntryPoint.cs`, `Core/MediationDebuggerOverlay.cs`, `Platforms/{Android,iOS}`, `Core/BannerHost.cs`). While tier 2 is not yet wired, those files stay `planned`; the ratchet keeps this visible.

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
- **Pragma elimination**: the design requires deleting every `# pragma: no cover` marker (18 across `ML/**` plus 1 in `services/fraud-inference/main.py:33` and 1 in `services/inference-ml/main.py:24`; all anchored and frozen in `tools/coverage/policy-baseline.json`) and writing real tests for those lines. The dogfood-audit greps for any new marker and fails. Where a marker guarded genuinely unreachable code (e.g. a defensive `except` around `sys.exit`), the code is restructured to be injectable and tested.
- services/fraud-inference, inference-ml: `pytest tests --cov=main --cov-branch --cov-report=lcov --cov-fail-under=100` (FastAPI app exercised via TestClient + adversarial payloads; prom-client registry reset between tests to avoid the cross-test coupling that forced `pragma` markers).

### 3.9 Rust — pipelines `rust-coverage.yml` and `apex-rs-coverage.yml`

`services/mail-server` runs `cargo llvm-cov --workspace --all-features --lcov` with `--fail-under-lines "$FLOOR"`, the floor read from `tools/coverage/floors.txt`. Branch parity comes from the lcov adapter (`tools/coverage/lcov-gate.mjs`) requiring BRH==BRF on every first-party `SF` record; the proto-generated files under `mail-proto` are `GENERATED`-excluded with the generator anchor `proto/mail.proto`.

The seven product crates (`apex-core`, `apex-connectors`, `apex-audit`, `apex-proof`, `apex-autopilot`, `apex-billing`, `apex-api`) run one matrix lane each in `apex-rs-coverage.yml`: per-crate llvm-cov, `cargo test -p`, clippy `-D warnings`, `cargo fmt --check`, and the same lcov gate.

### 3.10 Flutter — pipeline `sdk-flutter-coverage.yml`

`flutter test --coverage` + `tools/coverage/lcov-gate.mjs coverage/lcov.info 100` (lines; Flutter's lcov lacks branch records — the gate accepts absence of BRH/BRF but fails below 100 lines; branch parity is a documented phase-3 upgrade via `coverage` pkg `branch: true`).

### 3.11 Shell — pipeline `scripts-shell-coverage.yml`

- `bats-core` suites per script group, with `kcov` line and branch coverage aggregated through the lcov gate at the shell floor.

- Non-interactive behavior comes from extracting pure logic into `scripts/lib/*.sh`, sourced by the entry scripts (mandated refactor for the 12 largest scripts, e.g. `scripts/deploy-all.sh`).

- Destructive scripts (deploy, ssh, point-in-time ledger) run only in `--dry-run` mode under test; each entry script must honor `DOGFOOD_DRY_RUN=1`, which short-circuits side effects. A contract suite greps for side-effecting commands outside the dry-run guard.
- `set -euo pipefail` presence is a contract check per script (`shell-gate`).

### 3.12 What is intentionally NOT line-covered

- `src/types/*.d.ts`, protobuf-generated code, `docs/**` (contract via §5.10), `data/`, `models/` (data artifacts), `sdk/samples/**` + `sdk/adapter-samples/**` + `Test Apps/**` (sample apps: build+smoke contract `S-SAMPLE`), lockfiles (build artifacts), `.junie/`, images. Everything else is owned.

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

Adversarial axes applied to every domain: malformed input (fuzz/property), resource exhaustion (bombs, unbounded growth), race and ordering (concurrency, retries, idempotency), and authority (authn/z bypass, privilege escalation, tenant isolation). The remaining axes: time (skew, DST, expiry), observability gaps (silent failure must still emit metrics/alerts), and state corruption (crash-between-write, replay, double-spend).

---

## 5. Adversarial expectation catalog

Format: every row is given/when/then with an anchor. `A#` ids are stable and referenced by suite names. Rows marked `AUDIT-REQUIRED` require phase-0 behavior recording before implementation (§0.2). This catalog covers the crown-jewel cases; every remaining owned file inherits its domain's axes and MUST appear in its suite's case list (the manifest ties file→suite; the suite case list ties case→file).

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
| B3 | Adyen webhook valid sig, `t` 10 min old | process | rejected (Adyen tolerance 5 min). **IMPLEMENTED in this change**: tolerance pinned explicitly to 300s in `constructEvent` | `backend/src/routes/webhooks.routes.ts:86` |
| B4 | invoice period boundary | usage at exact `periodEnd` timestamp | attributed to one period only; rounding = banker's rounding to cent, no float accumulation | `services/billing/platformFeeService` (top-level backend/services/billing/) AUDIT-REQUIRED |
| B5 | payout already in `paid` state | duplicate payout request | 409 or no-op; ledger invariant `sum(entries)==0` holds | AUDIT-REQUIRED |

### 5.3 Backend RTB/auction (suite `S-BACKEND-RTB`)

| id | given | when | then | anchor |
|---|---|---|---|---|
| R1 | OpenRTB bidrequest | `imp[]` empty / `bidfloor: -1` / `tmax: 0` | deterministic rejection (400) with no DSP fan-out | `backend/src/services/rtbEngine` AUDIT-REQUIRED |
| R2 | DSP responds after internal deadline | hedge race | winning path returns before deadline; late response discarded and counted in timeout metrics | `backend/src/services/rtbEngine`, Go `backend/auction/internal/bidding` (hedge tests exist: anchor `backend/auction/internal/bidding/*_test.go`) |
| R3 | waterfall with cycling adapters A→B→A | walk | duplicates are deduplicated before the walk by priority (highest kept) and logged; repeated ids cannot burn an attempt slot, the indexed loop still bounds attempts, and the fallback wins fill. Shipped in commit `9f1a944` | `backend/src/services/waterfallService.ts` |
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
| K4 | TCF string | malformed/oversized consent string | parser fails closed (no consent), never throws across API boundary. **IMPLEMENTED in this change**: `TCFParser.kt` now decodes the real TCF v2 bitfield (version==2, all 3 segments base64url-decoded, purpose-1 bit read MSB-first) and returns consent=false for everything it cannot decode — the previous "has a dot" heuristic failed OPEN | `sdk/ctv/android-tv/src/main/kotlin/com/rivalapexmediation/ctv/consent/TCFParser.kt` + tvos `TCFParserTests` extension |
| K5 | Swift strict concurrency | ad request cancelled mid-flight | no detached-task leak; actor state consistent (`-strict-concurrency` build already on) | `sdk/core/ios/Package.swift` (StrictConcurrency) |
| K6 | Web SDK SSAI parser | manifest with duplicate `#EXT-X-DISCONTINUITY`, zero-duration segments | parsed deterministically or rejected; no NaN in timeline. **IMPLEMENTED in this change**: all 8 bare `parseFloat` sites in `SSAIManifestParser.ts` replaced with fail-closed `safeNumber` (malformed attrs → documented defaults, NaN eliminated) | `sdk/web/src/ssai/SSAIManifestParser.ts` |
| K7 | Unity tier without Unity | `UNITY_RUNNER` unset | Tier-1 gate runs the suite for real (self-skip inverted: `UNITY_RUNNER=true` + `Tests/ApexMediation.Tests.csproj`); an empty/skipped run cannot produce the lcov the gate requires | `sdk/core/unity/Tests/ApexMediation.Tests.csproj` |

### 5.9 Contract domain suites

| id | target | adversarial check | gate |
|---|---|---|---|
| S-SQL | 109 migration files + `tools/sql` | M1/M2/M3 above | `backend-coverage.yml` step |
| S-IAC | monitoring rules, helm, terraform, nginx, caddy, compose | `promtool test rules monitoring/alerts.test.yml` (created in phase 1 with adversarial alert-firing cases), `helm lint && helm template \| kubeconform -strict`, `terraform validate`, `nginx -t`, `caddy validate`, `docker compose config` | `contract-coverage.yml` |
| S-OPENAPI | `backend/src/openapi/billing.yaml` | redocly lint + drift vs baseline + every documented route exists in code (and vice versa) | `contract-coverage.yml` |
| S-CI | `.github/workflows/*.yml` + `.woodpecker/*.yml` | every referenced path exists (`ci-reference-integrity`), no `|| true` outside allowlist, required-checks parity, woodpecker gate set covers all domains | `contract-coverage.yml` + `dogfood-audit.yml` |
| S-SAMPLE | `sdk/samples/**`, `adapter-samples`, `Test Apps` | all sample apps compile (`build` contracts) + smoke boot | `contract-coverage.yml` |
| S-CONTENT | `tools/content-guard.js` | guard fed adversarial provider strings (homoglyphs, split tokens) — guard itself must classify correctly | `contract-coverage.yml` |

### 5.9.1 Additional contract suites, shipped

| id | target | adversarial check | gate |
|---|---|---|---|
| S-I18N | locale trees under console and website `i18n` | every locale carries exactly the reference key set; empty strings, arrays and non-string leaves fail closed | `contract-coverage.yml` step `i18n-parity` |
| S-CSS | console, website and site stylesheets | balanced braces and quotes, no empty rules, `!important` only with an in-rule justification, lowercase hex, size budget; the console theme block was refactored to remove fifteen unjustified flags and website keeps only the audited reduced-motion pattern | `contract-coverage.yml` step `css-contract` |
| S-SAMPLE | six sample projects | structural contract from `samples-contract.json`; each sample carries its buildable shape and the web sample has a build script and entry page | `contract-coverage.yml` step `samples-build-contract` |

### 5.10 Docs domain (suite `S-DOCS`)

Linkcheck is real: `tools/coverage/docs-links.mjs` resolves every relative markdown link and every #anchor against document headings, with a ratchet baseline of 171 frozen legacy rows across 51 documents (rows may only shrink; a stale row fails). Adversarial case `S-DOCS-2`: break a relative link or move a heading, and docs-links fails. The freshness contract stays: every doc referencing an npm script, Makefile target or env var references something that exists. The prose ratchet (M-10) scans `docs`, `website/src`, `console/src`, `tools/coverage`, `tools/docs-lint`, `tools/ui-ux`, `site`, `rust`, and `.woodpecker` against a committed baseline total of 8005.

### 5.11 Findings implemented by the dogfooding run itself (this change)

The first execution of this stack (Unity tier-1, promtool, shell/bats, contract checks, and the four design audits) surfaced pre-existing defects. All were closed in the same change (never weakened or deleted):

| id | defect found | fix |
|---|---|---|
| U-1 | `sdk/core/unity/Runtime/Core/MediationSDK.cs` did not parse at all (methods spliced into `InternalShow`'s body) — the entire Unity package had never compiled in CI | method bodies restored; missing `GetCircuitBreaker` body implemented from call sites; Unity tier-1 now compiles and 25/25 tests pass |
| U-2 | `FallbackWaterfall.CreateSource` had an optional parameter before a required one (CS1737, uncompilable in Unity too) | 3-arg overload added + 4-arg shape preserved |
| U-3 | `UsageMeter.cs` used LINQ without `using System.Linq` (uncompilable under Unity's compiler) | using added |
| U-4 | `RenderableAd` had no constructor for the pre-`format` test API | backward-compatible overload added |
| U-5 | `MediationSDK.Shutdown()` leaked pacing marks, circuit breakers, and cached ads across lifecycles (the adversarial network suite caught it: timeouts surfaced as "pacing") | Shutdown now resets all three; `AdCache.Clear()` added |
| U-6 | `scripts/ops/deploy_analytics_pipeline_hotfix.sh` was unparseable (embedded quotes broke the outer string); would have failed deploy | rewritten with quoted-heredoc payloads (rollback.sh pattern); validated with `bash -n` under bash 5 |
| U-7 | 51 shell scripts lacked `set -eu` and/or a dry-run guard | guards + strictness implemented script-by-script; bats suite now enforces both invariants |
| U-8 | `tools/content-guard.js` was not importable (top-level side effects), so it could not be dogfooded | refactored to pure predicates + main-guard; adversarial payload matrix runs in the contract pipeline |
| U-9 | 4 workflows referenced the pre-2025 `sdks/`/`packages/`/`Packages/` layout that has never existed (web-sdk, kill-switch web lane, docs-deployment, unity-ci/unity-sdk triggers) | paths repaired to the real `sdk/` tree; remaining legacy workflows baselined in `ci-ref-baseline.json` with ratchet |
| U-10 | `Il2CppCompatibilityTests` used a removed NUnit-4 API | modernized to `Assert.That(..., Is.Not.Null)` with real public API types |
| U-11 | `website/src/__tests__/security.headers.test.ts` was a 1-line stub that could never assert | implemented as the full TS twin of the JS suite |

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
| M-6 | CI reference integrity (all paths in workflows exist) | `tools/coverage/contract-checks.mjs ci-reference-integrity` |
| M-7 | every test file contains at least one assertion (kills always-green tests) | meta test scan |
| M-8 | self-coverage: the gate tooling runs under `npm run test:cov` in `tools/` | c8 over `coverage/*.mjs`, `site/*.mjs`, `ui-ux/*.mjs`, `content-guard.js`; the `coverage-gate.mjs --domain js-tools` step |
| M-11 | walker parity: no git-tracked file or symlink may be invisible to the manifest | `dogfood-gates.mjs walker-parity` compares `git ls-files -s` against `coverage-manifest.json`; gitlinks are accounted, case-folded names are noted, anything else fails |
| M-9 | zero scaffold markers on the new-model surfaces | `dogfood-gates.mjs stub-ban` over `rust/`, `tools/coverage/`, `tools/docs-lint/` |
| M-10 | prose ratchet: every scanned row at or below its committed baseline; adoption is one-shot | `docs-lint.sh --source --baseline` plus `--integrity` in `dogfood-audit.yml` |

### 6.2 Why mutation testing is the "correct expectations" oracle

A passing suite only proves expectations hold for the written inputs. Mutation sampling (StrykerJS/mutmut) mutates production code and requires the suite to kill mutants. An expectation that does not pin behavior lets mutants survive; the audit scores kills and fails below floor. That is the only mechanical proof that expectations are *correct and tight*; phase 3 therefore started at a 60% kill rate with a ratchet toward 80%. Phase 3 has started: `apex-tools mutation-smoke` applies six hand-picked mutants across money and validation in a throwaway copy of apex-core, demands the suite kill each one, and writes `tools/coverage/mutation-results.json`. The floor was raised to 100% on 2026-10-14 (§9.7) because the sample kills six of six. The StrykerJS/mutmut rollout continues in later phases toward the 80% target.

---

## 7. Woodpecker design

### 7.1 Server/runner

Self-hosted design in `infrastructure/woodpecker/`: docker-compose for server and agent on the existing Hetzner app-server pattern, plus a GitHub OAuth app integration. Commit-status reporting gives GitHub branch protection the same power as `required-checks.json`; the protected-branch contexts are `coverage-gate`, `dogfood-audit`, and `contract-coverage`.

### 7.2 Pipeline inventory (`.woodpecker/`, 17 configs: 16 lanes plus the aggregate verdict)

Inventory note: the all-Rust tree runs six configs (five lanes plus the aggregate verdict); the multi-language inventory below is kept as design history and is superseded by §9.7.

| pipeline | covers domains | key services |
|---|---|---|
| `apex-rs-coverage.yml` | the seven apex product crates (one matrix entry each) | none (cargo-llvm-cov) |
| `backend-coverage.yml` | backend TS + SQL | postgres:16, redis:7 |
| `frontend-coverage.yml` | console, website, control-plane | none (jsdom/supertest) |
| `node-services-coverage.yml` | edge-worker, autopilot, sdk/web, sdk/react-native, tools JS | none |
| `go-coverage.yml` | 4 backend Go modules + quality Go | postgres (auction integration) |
| `sdk-android-coverage.yml` | Kotlin core + ctv | none (Robolectric) |
| `sdk-apple-coverage.yml` | iOS + tvOS | none (darwin agent) |
| `sdk-unity-coverage.yml` | C# tiers 1+2 | none |
| `sdk-flutter-coverage.yml` | Dart | none |
| `python-coverage.yml` | ML, fraud-inference, inference-ml, py-scripts | postgres (fraud) |
| `rust-coverage.yml` | mail-server | none |
| `scripts-shell-coverage.yml` | 64 sh + tools sh | none (kcov in-image) |
| `contract-coverage.yml` | SQL/IaC/OpenAPI/CI/content/samples/docs | postgres (SQL apply) |
| `dogfood-audit.yml` | manifest, meta M-1..M-10, stub-ban, docs-lint steps | none |
| `site-perf.yml` | site build, claims contract, byte budgets, Lighthouse | zola image |
| `ui-ux.yml` | rendered console pages: axe-core audit + structure contract | rust + node |
| `coverage-gate.yml` | aggregate verdict; `depends_on` the lanes above (§9.7 for the current five) | none |

All pipelines trigger on `push` and `pull_request` to `main`/`master` with **no path filters** in phase 0. Path-filtered parents that get skipped must not silently weaken the aggregate gate; filters are a phase-4 optimization behind verified `depends_on`-skip semantics. Secrets: none required for the gates. An optional `codecov_token` push-only upload step exists per pipeline; PR events never receive secrets, so uploads stay out of gate logic by design.

### 7.3 Existing `|| true` inventory (M-4, frozen with ratchet)

The legacy soft-fail inventory (36 occurrences across 12 workflows: `ci-all.yml` lighthouse/billing-e2e/website lanes, `ci.yml`, `compatibility-testing.yml`, `deploy-production.yml`, `ios-sdk.yml`, `ml-nightly.yml`, `nightly-billing-perf.yml`, `scripts-guardrails.yml`, `security.yml`, `synthetic-probes.yml`, `unity-sdk.yml`, `website-capture.yml`) is frozen in `tools/coverage/policy-baseline.json`. The policy gate fails on any new `|| true` on a test/coverage command, and on stale baseline entries, so each conversion must delete its entry. None of these may guard coverage numbers.

### 7.4 Rollout phases

- **Phase 0 (landed)**: Woodpecker infra design plus pipelines, manifest tooling, additive coverage configs, catalog, and meta gates. Gates ratchet from observed baselines; `coverage-floor.json` is seeded so the first run is green while every number stays pinned.

- **Phase 1**: implement suites per domain in ratchet order (smallest first: tools/js, shell lib extraction, python services, ML pragmas); floors +10/week until 100.

- **Phase 2**: mutation floors; instrumented-Android emulator lane; Unity tier-2 licensed lane.

- **Phase 3**: branch-parity for Apple (region gate), Flutter branch, path-filter optimization after `depends_on`-skip semantics verified.

- **Completion criterion**: `coverage-gate` green with every domain floor = 100 and manifest orphans = 0, for 14 consecutive days on `main`.

---

## 8. Self-verification protocol for this design

The design and the stack it describes are checked by four independent audits, run as read-only reviews alongside the implementation; every reported item landed in the commit series (§9):

1. **Woodpecker semantics audit** — every `.woodpecker/*.yml` and `infrastructure/woodpecker/*` checked against Woodpecker 2.x semantics (when/steps/services/secrets/depends_on/plugin settings) and against real repo paths/scripts/env vars.

2. **Completeness audit** — an independent file enumeration (find/ls, NOT the manifest tool itself) diffed against the manifest domain rules to find any unclassified file/type/directory.

3. **Expectation correctness audit** — every anchored expectation in §5 checked against the cited source; mismatches or overclaims reported and addressed.

4. **Mechanism audit** — every gate command/flag/config (jest thresholds, pytest-cov, kover DSL, xccov fields, cargo-llvm-cov flags, go cover parsing, kcov, c8) verified to exist in the pinned tool versions.

Findings are resolved before this design is considered final; the remediation log is appended in §9.

---

## 9. Verification and remediation log

### 9.1 Design audits (4 independent read-only reviews)

- **Woodpecker semantics audit**: found 15+ issues across steps, services, and images (top-level `environment` unsupported, matrix grading-cache build, missing `gradlew` in both Android modules). Further items: a wrong Unity csproj path, missing coverlet, `podspec`, runtime-image gaps, a `gixy` pin, promtool extraction pollution, `apt-get` in the kcov image, missing pytest-cov, `--require-branches` fail-open, and a `get()` parsing bug. All blocker and major items were resolved in the pipelines and configs.

- **Completeness audit**: enumerated real repo files against the classifier and found around 40 gap classes (root-anchored patterns, missing extensions, order hazards, `tools/coverage` self-skip, `**/test/**` over-match, `.md` nesting). The classifier was rewritten; the manifest now classifies 2,904 files with 0 unclassified.

- **Expectation-correctness audit**: verified every anchored catalog row. Corrections landed for the Unity csproj path, the ML pragma count (18 markers across 11 files, with 13 files frozen in the policy baseline), the `|| true` counts (36 across 12 files), and the `webhooks.routes.ts:86` anchor. Also corrected: the absent backend `README.md`, the `cron/create_future_rollup_partitions.sh` name, the `TCFParser` failing-open behavior, and the `sales-scheduler.ts` path. Rows contradicted by the code were made true by implementation (§5.11, B3, R3, K4, K6) rather than by weakening.

- **Mechanism audit**: verified jest/pytest/kover/llvm-cov/cargo-llvm-cov/kcov/c8 semantics. The Kover 0.8 DSL conflict was resolved against the 0.8.3 sources jar (`coverageUnits` plus `aggregationForGroup`; `CounterType` does not exist). The lcov branch check now fails closed, unknown domains in `coverage-gate.mjs` are fatal, and the c8 include/report-dir, `assertion-presence` regex, and `dogfood-gates` flag-parse defects were closed.

### 9.2 Dogfooding execution (local; the first landing of the stack)

| check | result |
|---|---|
| `node --test tools/coverage/test/` | 22/22 pass |
| all tool suites (`coverage`, `site`, `ui-ux`) | 42/42 pass |
| manifest build | 2,904 files, 0 unclassified |
| orphan gate / floor-ratchet / floors-sync | pass (30 domains in lockstep) |
| assertion-presence (M-7) | pass (10 always-green candidates resolved: 4 got real assertions, 6 reclassified as support files) |
| policy gate `--shell` + `--full` (M-3/M-4) | pass (51 scripts hardened, 22 skip markers implemented; baseline frozen at 13 pragma files and 12 `|| true` files) |
| contract: ci-reference-integrity | pass (37 workflow files; 6 legacy refs baselined) |
| contract: openapi-parity | pass (5/5 paths, mount-prefix aware) |
| contract: content-guard adversarial matrix | pass (6 payload cases) |
| contract: samples / docs-references | pass |
| promtool `test rules monitoring/alerts.test.yml` (real promtool v2.53 in docker) | success (3 alert state-machine cases including exact labels and annotations) |
| Unity tier-1 `dotnet test` (real dotnet) | 25/25 pass; lcov emitted (47 files, 31% baseline) |
| `.woodpecker` configs parse | 17/17 |

### 9.3 Residual items (by design, tracked by ratchet)

- Phase-1: delete the 13 pragma files' markers and convert the 12 `|| true` inventories; stale baseline entries fail the policy gate until they are removed.
- Phase-1: rebuild the two legacy workflows behind `ci-ref-baseline.json`, implement the remaining `AUDIT-REQUIRED` catalog rows with the §0.2 protocol, and populate `suites.json` so `orphanFloorPercent` can begin ratcheting below 100.
- Phase-2: Unity tier-2 licensed lane, instrumented Android emulator lane, mutation floors.
- Console layout checks: `color-contrast` and `target-size` run outside axe; the palette ratios are pinned in the apex-api Rust test and the site contrast runs in the Lighthouse lane (documented in `tools/ui-ux/console-a11y.mjs`).
- Live-sandbox confirmation: the four LevelPlay endpoint paths are `VerifyLive`; autopilot writes through them stay blocked until the live-integration lane confirms them (apex-core `endpoint.rs` and `levelplay.rs`).
- Not locally executable here: the Woodpecker server itself (the local runs exercise the same commands) and the GitHub-Actions-only toolchains (Android, iOS, Flutter, Kover gradle). Their commands and configs were validated statically; where a local tool existed (dotnet, promtool, bash, node, cargo), the exact commands ran green.

### 9.4 Rust verifier waves and pinned regressions

Three independent verifier waves found critical and major defects in the Rust product stack after it landed. Each reported item carries falsifier evidence and closed with a pinned regression test in the commit series below.

| wave | defect | pinned regression | commit |
|---|---|---|---|
| 1 | rollback target constraint: a rollback could point at another platform or ad unit, or at a non-change record | `rollbacks_must_target_the_same_platform_and_ad_unit_change` | `9f1a944` |
| 1 | canonical-byte injection: attacker-added record fields and re-encoded signature bytes passed the structural checks | `injected_unknown_fields_are_rejected`; `hex_case_and_whitespace_edits_are_rejected` | `9f1a944` |
| 1 | audit NITs A5-A9: zero-floor semantics, the overflow story in unavailable-eCPM reasons, digest sensitivity, and a tautological assertion | pinned regressions in `apex-audit/src/tests.rs` | `28ec482` |
| 2 | partial-charge reporting: a mixed sub-cent invoice was charged item by item before the error surfaced | `mixed_sub_cent_invoice_is_refused_before_any_request` | `1666051` |
| 2 | falsified form encoding: billing claimed form encoding while sending JSON, and the encoder was dead | `stripe_item_body_is_pinned_form_encoded` | `1666051` |
| 3 | verify and re-read time of check: the ledger could be swapped between verification and the second read | `ledger_read_path_is_pinned_to_a_single_read` | `3def08a` |
| 3 | ignored Adyen tolerance: the webhook route used state tolerance in place of the configured client policy | `webhook_client_tolerance_beats_a_wider_state_tolerance`; `webhook_client_tolerance_beats_a_narrower_state_tolerance` | `3def08a` |

### 9.5 Current green state

All checks below ran locally on 2026-10-13 from the repository root.

| check | result |
|---|---|
| Rust workspace (`cargo test --workspace`) | 347 tests, 0 failed |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --all --check` clean |
| apex crate coverage lanes | 9 per-crate matrix entries in `apex-rs-coverage.yml` (measured floors, ratcheting; §9.10) |
| tool suites (`npm test` under `tools/`) | 42/42 pass (coverage 22, site, ui-ux) |
| docs-lint | total at or below the committed baseline of 8005; every scanned row at or below its row |
| docs-lint one-shot adoption | self-test all pass (10 cases) |
| coverage manifest | 2,904 files, 0 unclassified |
| floors | 30 domains in lockstep (`coverage-floor.json` ↔ `floors.txt`) |
| stub-ban | no scaffold markers in `rust/`, `tools/coverage/`, `tools/docs-lint/` |
| aggregate verdict | pass locally (`aggregate-coverage-verdict.mjs`) |
| site claims contract | 21/21 claims anchored and present on the site |
| site perf budget (`tools/site/perf-budget.mjs`) | 7 pages, css 3213B (budget 12000B), zero JS, robots+sitemap present |
| console a11y (`tools/ui-ux/console-a11y.mjs` over `render-fixtures` output) | axe-core WCAG 2.0/2.1 A+AA pass over 4 rendered pages; structure contract clean |
| Woodpecker configs | 11 configs parse; 10 lanes wired into the aggregate (§9.10) |

### 9.6 Blind-spot sweep results (all gates executed locally, in order)

| gate | result |
|---|---|
| tools test suites | 59/59 |
| manifest | 2,946 files, zero unclassified |
| walker parity | PASS, one gitlink accounted and one case-folded name noted |
| orphan gate | PASS at the 100% phase ceiling |
| floor ratchet and floors sync | PASS, 30 domains in lockstep |
| assertion presence | PASS |
| stub ban | PASS over rust, tooling and site |
| mutation floor | PASS, sample kills 6/6 against a 60% floor |
| policy gates, shell and full | PASS |
| contract checks: ci-reference, openapi, content guard, samples, docs references | PASS |
| docs links | PASS, 171 frozen rows |
| i18n parity | PASS |
| css contract | PASS |
| docs-lint prose ratchet | OK, 7,941 at or below 8,005 with every row in budget |
| Woodpecker YAML | all valid |
| rust workspace | 366 tests, clippy warnings-free, formatted |
| site | perf budget PASS, 21 claims PASS |
| console a11y | WCAG 2.0 and 2.1 A plus AA clean |
| aggregate verdict | PASS |

### 9.7 All-Rust conversion sweep (2026-10-14)

Second execution of the stack, run adversarially against the all-Rust tree. Every finding was
patched in the same change; the table is the complete list.

| area | defect | patch |
|---|---|---|
| gate stack | `walker-parity` skipped tracked files missing from disk as "pending deletions" (2,361 skipped on the live tree), so the purge was invisible to the manifest proof | index/tree divergence now fails with an actionable list; the purge deletions were staged, and parity verifies every tracked path |
| gate stack | `assertion-presence` filtered by classifier kind, so every inline Rust test module was invisible (4 files checked, all JS-era) | content-based scan: any code file defining test cases must carry an assertion (51 files) |
| gate stack | the assertion regex knew `assert.` and `\bassert\b` but not the Rust macros `assert_eq!`, `assert_ne!`, `matches!` | regex extended; a pinned unit test covers the macros |
| gate stack | `mutation-floor` returned success when the results file was missing or empty | missing file and empty samples fail closed |
| gate stack | `floor-ratchet` never compared the live domain table against the last history entry | a pinned domain can no longer disappear or drop below its recorded floor |
| gate stack | `yaml-policy` excluded the whole apex-tools crate from the pragma scan and only knew JS skip markers | only the file defining the pattern is exempt; `#[ignore]` is scanned for Rust |
| gate stack | `stub-ban` scanned neither `services/` nor `.woodpecker/`, hiding a real TODO in the outbound queue | both roots scanned; the gRPC `Status::unimplemented` message text stays allowed |
| gate stack | `docs-lint` resolved baselines against the process cwd, so the lane (which runs from `rust/`) could never find one; the lane also named the links baseline | relative targets resolve against the repo root; the lane names `docs-lint-baseline.txt` |
| gate stack | `docs-lint` silently dropped `.woodpecker/` and the root README from the scan when the README title changed | scan roots are unconditional; the baseline was regenerated with zero-count rows only |
| gate stack | `docs-links` treated a leading-slash site route as a document path, and absolute paths could escape `PathBuf::join` | site routes map to content pages, section indexes and static files; escaping `..` links fail |
| gate stack | the standalone `lcov-gate` passed a record set with zero executable lines; `go-func-gate` passed empty artifacts and never parsed integer rows | both fail closed; the Go gate parses rows through the shared parser |
| gate stack | `docs-references` was vacuous without any package.json, and the content guard only tested its own memory | docs references ratchet through a committed baseline and cover make targets; the guard scans the site |
| pipelines | the apex lane used `/w/...` paths, wrote the lcov where the gate did not read it, and `cd rust/services/mail-server` for the mail-server entry | lane rewritten: per-crate package runs, a workspace entry for mail-server, a built release gate binary, real paths |
| pipelines | ui-ux handed `target/ui-fixtures` to the audit while the render step wrote `rust/target/ui-fixtures` | both steps use the rendered path |
| pipelines | no lane ran `yaml-policy`, `docs-links`, `i18n-parity` or `css-lint` | wired into dogfood-audit and contract-coverage |
| pipelines | the site lane ran `console-a11y` against a built site while the audit read only top-level html, so the 7-page build read as 2 pages and failed | page discovery now walks nested directories and reports repo-relative paths; a pinned unit test covers the nested case |
| pipelines | the terraform step ran `validate` without `init`, and the production compose step had no `POSTGRES_PASSWORD` in the environment | the lane initialises providers first and passes a config-only password value |
| mail-server | the workspace did not build: a nonexistent `trust-dns-resolver` version, a `[[bin]]` pointing at a missing file, missing sqlx/zstd/tokio-stream/rand dependencies, a build script writing generated code into the source tree, and 24 warnings under `-D warnings` | dependencies repaired, the build script delivers to OUT_DIR, CI installs protoc, crates gained library surfaces, the outbound queue gained a working `--send-to` path, tests and clippy and fmt are clean |
| repo state | `Logo.jpg` and `logo.jpg` were both tracked with identical content; the sanitized log sample was gitignored; `services/mail-server/target` had no ignore rule | one index entry removed, ignore negation and target rule added |
| ratchet | every floor sat at 0 and the orphan ceiling at 100, so the coverage gates could not fail | floors pinned to the integer floor of measured lines/functions per crate (9 to 99), orphan ceiling 0, mutation floor 100, floors.txt resynced |

### 9.8 Sweep results (all gates run locally, in order)

| gate | result |
|---|---|
| rust workspace | `cargo test --workspace` green; clippy `-D warnings` and `cargo fmt --check` clean |
| mail-server workspace | tests green; clippy `-D warnings` clean; fmt clean |
| manifest and walker parity | 566 paths classified, 0 unclassified, 0 invisible, 0 missing from disk |
| orphan gate | 0 orphans against a ceiling of 0 |
| floors | 9 domains in lockstep at measured floors; ratchet holds |
| assertion presence | 51 files define test cases, all assert |
| stub ban | clean over rust, services, coverage tooling and pipelines |
| policy gates | shell and full pass with an empty baseline |
| mutation smoke and floor | 6/6 killed, floor 100 |
| contracts | CI references, pipeline paths, content guard (site scanned), docs references, docs links, i18n, css all pass |
| docs-lint | 7,687 at the frozen baseline, every row in budget |
| site | perf budget pass, 21 claims pass, promtool alert tests pass |
| console a11y | the 4 fixture pages and the 7-page built site audited, structural WCAG checks clean |
| coverage | per-crate lcov gates pass against the new floors |
| aggregate verdict | pass, replaying manifest, orphan, ratchet, sync, walker and mutation invariants |

### 9.9 Post-sweep review hardening (2026-10-14)

An adversarial review of the sweep change found and fixed these defects before the change landed.

| area | defect | patch |
|---|---|---|
| pipelines | Woodpecker runs a step's commands as one `set -e` script, so the repeated `cd rust && ...` lines in multi-command steps aborted on the second command, and the ui-ux audit used a binary path relative to the changed cwd | every lane command is now absolute and idempotent (`cd "$CI_WORKSPACE"/rust && ...`, `cd "$CI_WORKSPACE" && rust/target/...`) |
| pipelines | with any step using `depends_on`, Woodpecker runs the rest in parallel, so walker parity and the orphan gate raced the manifest producer | both steps declare `depends_on: [manifest]` |
| repo state | the render-fixtures helper binary was untracked and ignored by `**/bin/`, so a clean checkout had no render target; the walker also skipped `bin` directories, hiding the file from the manifest | `.gitignore` negates the source path, the file is tracked, and the walker no longer skips `bin` or `obj` |
| mail server | routing the SMTP session through the parser narrowed `MAIL FROM:` to its first token, breaking the valid spaced address form | the session keeps the full remainder after the verb; two pinned session tests cover the spaced forms and the size or BINARYMIME rejections |
| gates | the new Rust skip scan ignored the attributed `#[ignore = "reason"]` form and the `dogfood:skip:` marker self-exempted the gate file | the scan matches `#[ignore\b` and excludes the gate file explicitly |
| gates | the content guard and the reference ratchet silently skipped unreadable files | both fail closed on unreadable input |
| gates | site-route links to the site root or to a page bundle were reported missing | route candidates include `_index.md` and `index.md` bundles; a pinned test covers both |
| gates | three copies of the `count path` / `TOTAL n` baseline grammar had drifted | one shared parser and comparator in `baseline.rs`, used by all three ratchets |
| gates | the Makefile and lanes duplicated gate lists with nothing checking them | `apex-tools` exposes its subcommand inventory, and the aggregate verdict fails when a lane or Makefile references a subcommand that does not exist |
| repo state | the dev compose published Redis without authentication and hardcoded the Postgres password; the monitoring runbook referenced a deleted script, the suites registry was orphaned, and the design doc inventories contradicted the six-config set | loopback binds and requirepass restored, env-overridable credentials, the runbook describes the render step, the orphaned registry is deleted, and the stale inventory lines point at §9.7 |

Re-measured after hardening: apex-tools at 56.43 percent lines and 58.78 percent functions, mail-server at 25.50 and 16.88, both above their floors; the full audit family and the site gates run green.

### 9.10 CI coverage sweep (2026-10-14)

The lane set was rebuilt to cover 100% of the tree with correct expectations.

| lane | surface | gate |
|---|---|---|
| `apex-rs-coverage` | every Rust crate plus the mail-server workspace | llvm-cov, tests, clippy, fmt, lcov floor |
| `dogfood-audit` | whole tree | manifest, walker parity, orphan, ratchet, sync, assertion presence, stub ban, policy, mutation, docs links, prose |
| `contract-coverage` | monitoring, terraform, proxies, compose, CI references, pipeline paths, content guard, docs references, samples contract, i18n, stylesheet | per tool gate |
| `site-perf` | the built Zola site | claims, byte budgets, structural checks |
| `ui-ux` | the rendered console pages | structural accessibility audit |
| `sdk-node` | web and react-native SDKs | jest, lint, tsc, bob build, lcov floor |
| `sdk-android-coverage` | Kotlin core and TV SDKs, both sample apps | kover line and branch floor, sample builds |
| `sdk-apple-coverage` | iOS, adapter dev kit and tvOS packages | swift test, llvm-cov lcov floor |
| `sdk-unity-coverage` | tier-1 Unity runtime | dotnet test, coverlet lcov floor, package size constraints |
| `sdk-flutter-coverage` | flutter SDK | analyze, test, lcov floor |
| `coverage-gate` | every lane | aggregate verdict replaying the invariants |

Measured floors added to the ratchet: android-core 32, android-ctv 11,
web-sdk 73, react-native-sdk 22. The unity, flutter and apple domains start
at 0 because their coverage artifacts need the lane toolchains; the lanes
still fail when tests fail.

Defects the sweep fixed, each found by actually running the platform suites:
assertion markers for `MAIL FROM: <addr>` style regressions, the auction
request envelope drift (`request_id`, `user_info` against the pinned tests),
the missing OM SDK test stubs, the Kotlin `SSAIError` visibility clash, the
react-native bob build configuration, the adapter dev kit package identity
cycle, the sample app repository mode clash and the unescaped XML ampersand.

### 9.11 Post-audit closure (2026-10-14)

The full repository audit (audit.pdf) directed parking the legacy mediation
SDK tree: it now lives under `future/sdk` with a marker README, classified as
`FUTURE_SURFACE` so the gates prove the files exist without treating them as
product code. The standalone mail-server workspace was removed, which
contradicted nothing in the no-SDK product plan. The five SDK lanes and the
mail-server matrix entry left with them; the lane set is now `apex-rs-coverage`,
`dogfood-audit`, `contract-coverage`, `site-perf` and `ui-ux`, plus the
aggregate verdict. Policy domains, suite claims and floors were pruned in the
same change with a new ratchet history entry. The monitoring stack now watches
`apex-api` only. The pipeline inventory check now fails in both directions, so
a subcommand with no caller cannot survive.

### 9.12 Apex Lift Engine (2026-10-14)

The lift engine design spec (Oct 14, 2026) is implemented as the new
`apex-lift` crate and wired through the gate stack.

| area | implementation |
|---|---|
| Measurement | `stats.rs`: mean daily log ratio of revenue per user, one-sided t bounds (90% final, 99% interim, 90% upper for harm), block bootstrap cross-check, duration planner with the spec's table pinned |
| Windows | `window.rs`: three-day burn-in, two-pull finality within 0.5%, whole-week windows, incident exclusion, per-arm values (active users on LevelPlay, allocated share on MAX) |
| Guardrails | `guardrails.rs`: fast harm, confirmed harm, fill stops (2 points rewarded and interstitial, 5 points banner, two days running), publisher edit, split mismatch at p < 0.001, reporting gap, platform incident |
| Decision | `decision.rs`: the six keep conditions, the single 99% interim keep, the one pre-registered extension, and lambda equal to the lower bound |
| Billing | `billing.rs`: monthly fee over active changes with log-share splitting (the spec's worked example pinned to the micro), 12-month terms with leap-day clamping, override pro-rating, re-checks that can only lower the billed lift |
| Fallback | `switchback.rs`: balanced two-day blocks, never three in a row, billing only with six blocks per arm and a positive bound |
| Priors | `priors.rs`: skeptical normal prior, conjugate updates, Thompson-sampling score over the pre-registered scope revenue and contract rate |
| Records | `records.rs`: the six signed record types in spec order, canonical JSON with pinned SHA-256 |

Evidence: 98 crate tests green. Coverage measured 96.21 percent lines and
96.73 percent functions. Stable llvm-cov emits no branch records, so
branches stay ungated. Floors entry 96, suite claim, and a lane matrix
entry with a crate-scope assertion on the lcov artifact. Four site claims
anchor to the engine tests, and the pricing page now reads that monitoring
is an optional plan, so "no required subscription" is literal.

The engine itself now carries calibration, candidate generation with
worst-case loss, the planning policy, the monthly billing run, re-check
scheduling, the scoreboard, the record emission pipeline, the daily
settings watcher with override detection, the weekday-balanced switchback
planner, the impression-postback path with covariate variance reduction,
and the re-run kit with its CSV template.
