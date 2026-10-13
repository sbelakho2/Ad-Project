# Woodpecker CI: the coverage gate stack

Each `*.yml` file in this directory is one Woodpecker pipeline (name = filename without
extension): 16 lanes plus the aggregate verdict in `coverage-gate.yml`, 17 configs in total.
Trigger: `push` + `pull_request` on `main`/`master`, no path filters (deliberate: see
`docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md` §7.2).

Lanes:

- `apex-rs-coverage.yml`: the seven Rust product crates, one matrix entry per crate.
- `backend-coverage.yml`, `frontend-coverage.yml`, `node-services-coverage.yml`: TypeScript domains.
- `go-coverage.yml`, `python-coverage.yml`, `rust-coverage.yml`: Go, Python, and the mail-server Rust workspace.
- `sdk-android-coverage.yml`, `sdk-apple-coverage.yml`, `sdk-flutter-coverage.yml`, `sdk-unity-coverage.yml`: the SDK languages.
- `scripts-shell-coverage.yml`, `contract-coverage.yml`: shell code and the non-line contracts.
- `dogfood-audit.yml`: manifest, floors, policy bans, stub-ban, docs-lint, self-coverage.
- `site-perf.yml`, `ui-ux.yml`: the Zola site budgets and the rendered-console accessibility audit.
- `coverage-gate.yml`: the aggregate verdict; it depends on the 16 lanes and fails the build when any domain sits below its floor in `tools/coverage/coverage-floor.json` (ratchet-only file).

Floors start at observed baselines (green day one) and may only rise; the final phase requires
every floor = 100. Server/agent deployment design: `infrastructure/woodpecker/README.md`.

Pipelines intentionally contain **no secrets in gate logic**: gates run identically on PRs
(Woodpecker does not expose secrets to PR events; nothing here needs them). Floors are
single-sourced from `tools/coverage/coverage-floor.json` and mirrored to `tools/coverage/floors.txt`
(grep-readable inside any container image) by `tools/coverage/sync-floors.mjs`; the dogfood audit
fails if they drift or if history regresses.

Locally verified before commit (see design doc §9.5): 347 Rust workspace tests with clippy `-D
warnings` and `cargo fmt --check` clean; 42 tool tests; the docs-lint gate OK against the 8005
baseline. The manifest holds 2904 files with 0 unclassified; the stub-ban gate passes; the
aggregate verdict passes locally.
