# Woodpecker CI: the gate stack

Each `*.yml` file in this directory is one Woodpecker pipeline: six lanes
plus the aggregate verdict in `coverage-gate.yml` (seven configs). Trigger:
`push` and `pull_request` on `main` and `master`, with no path filters
(deliberate: see `docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md`
section 9.10).

Lanes:

- `apex-rs-coverage.yml`: one matrix entry per Rust crate, each running
  llvm-cov, tests, clippy and fmt, then the lcov floor.
- `dogfood-audit.yml`: manifest, walker parity, orphan ceiling, floor ratchet
  and sync, assertion presence, stub ban, policy bans, mutation smoke and
  floor, docs links, prose ratchet.
- `dogfood-live.yml`: the live half of the adversarial dogfooding system: the
  three services booted against a real ClickHouse `service:` container, the
  live-gated `apex-analytics`/`apex-api` suites finally executing, then
  `tools/dogfood/live.py` running every required probe cell from
  `tools/dogfood/surfaces.json` and the ratchet baseline. The static catalog
  gate (`apex-tools dogfood-plan`) runs here too, so the lane is not
  redundant in a store-less build.
- `contract-coverage.yml`: monitoring rules and their tests, the edge config
  (validate plus `infrastructure/caddy/contract.sh`), CI reference integrity,
  pipeline paths, content guard, docs references, stylesheet lint.
- `site-perf.yml`: Zola build, claims contract, byte budgets, structural
  checks on the built output.
- `ui-ux.yml`: the rendered publisher console pages and the rendered
  operator control-plane pages checked by the structural accessibility
  audit.
- `coverage-gate.yml`: the aggregate verdict. It depends on every lane and
  replays the manifest, orphan, ratchet, sync, walker, mutation and pipeline
  inventory invariants before reporting.

Floors are single-sourced from `tools/coverage/coverage-floor.json` (ratchet
only) and mirrored to `tools/coverage/floors.txt` by `apex-tools sync-floors`;
the dogfood audit fails when they drift or when a code domain has no floor.

The local equivalent of the audit family is `make dogfood`. Every gate is a
subcommand of the `apex-tools` binary, and the pipeline inventory check fails
on both directions: unknown subcommands and subcommands with no caller.
