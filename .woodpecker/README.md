# Woodpecker CI — 100% coverage gate stack

Each `*.yml` file in this directory is one Woodpecker pipeline (name = filename without extension).
Trigger: `push` + `pull_request` on `main`/`master`, no path filters (deliberate: see
`docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md` §7.2 — path-filtered parents that get
skipped must not silently weaken the aggregate gate).

- Every pipeline enforces its domain's coverage floor INSIDE the pipeline (fail-fast, no report-and-pray).
- `dogfood-audit` builds the repo-wide manifest (`tools/coverage/build-coverage-manifest.mjs`) and enforces
  orphan/meta rules M-1..M-8.
- `coverage-gate` aggregates: it depends on all coverage pipelines and fails the build if any domain is
  below its floor in `tools/coverage/coverage-floor.json` (ratchet-only file).

Floors start at observed baselines (green day one) and may only rise; the final phase requires every
floor = 100. Server/agent deployment design: `infrastructure/woodpecker/README.md`.

Pipelines intentionally contain **no secrets in gate logic** — gates run identically on PRs (Woodpecker does not expose secrets to PR events; nothing here needs them). Floors are single-sourced from `tools/coverage/coverage-floor.json` and mirrored to `tools/coverage/floors.txt` (grep-readable inside any container image) by `tools/coverage/sync-floors.mjs`; the dogfood audit fails if they drift or if history regresses.

Locally verified before commit (see design doc §9.2): the tools self-tests (16/16), the full manifest/gate stack, promtool alert tests, and the Unity tier-1 suite (25/25) all pass.
